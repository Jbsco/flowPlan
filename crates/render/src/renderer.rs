//! Phase 2 renderer: cel-shaded forward pass + screen-space sobel outline.
//!
//! Construction wires the wgpu pipelines (scene + outline), depth texture, vertex/index
//! buffers for the two meshes, their uniform bind groups, and the outline bind group.
//! `resize` recreates the depth view and outline bind group. `render` interpolates
//! between two `SimSnapshot`s and submits a scene pass followed by the outline pass.

#![deny(unsafe_code)]

use bytemuck::{Pod, Zeroable};
use flowplan_assets::{CpuMesh, Vertex};
use flowplan_sim::SimSnapshot;
use glam::{Mat4, Quat, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::debug_lines::{DebugLineBatch, DebugLinesRenderer};
use crate::instances::{InstanceGpuData, InstanceRenderer};

/// Depth format for the Phase 1 forward pass.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Perspective near-plane distance (meters).
const NEAR: f32 = 0.1;
/// Perspective far-plane distance (meters).
///
/// Tuned for the Phase 3.6.3c.5b streaming mesh half-extent (10 km).
/// Corner diagonal is ~14 km; 20 km clip provides margin so mesh never
/// visibly clips at the horizon. Performance-slider candidate for
/// Phase 4.8 settings UI.
const FAR: f32 = 20_000.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    view_pos: [f32; 4],
    // Inverse of `view_proj`. Sky pass uses it to reconstruct world-space view rays
    // from screen-space NDC without a dedicated skybox cubemap.
    inverse_view_proj: [[f32; 4]; 4],
}

/// Outline pass uniform. `vec4` holds `[viewport_w, viewport_h, threshold, strength]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct OutlineUniform {
    viewport_and_tunables: [f32; 4],
}

/// Depth-edge threshold (post-projection). Empirically tuned for the Phase 2 placeholder
/// scene; raise to suppress noise on steep terrain, lower to catch subtler creases.
const OUTLINE_THRESHOLD: f32 = 0.0008;
/// Opacity of detected outline pixels.
const OUTLINE_STRENGTH: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ModelUniform {
    model: [[f32; 4]; 4],
    normal_mat: [[f32; 4]; 4],
    tint: [f32; 4],
    // x = band count, y = ambient floor, z = rim strength, w = rim exponent.
    shading: [f32; 4],
}

/// Per-material cel-shading parameters.
///
/// Phase 2 Milestone 2.1: banded diffuse + rim light. Values are passed through to the
/// fragment shader. `bands` is clamped to at least 1.0 in the shader; `rim_exponent` is
/// clamped to at least 0.01 to avoid `pow(_, 0)` producing a flat sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadingParams {
    /// Number of diffuse bands. 3-4 is the design target per ROADMAP § 2.1.
    pub bands: f32,
    /// Floor of the banded diffuse term in [0, 1]. Higher values brighten shadows.
    pub ambient_floor: f32,
    /// Rim-light scalar added on top of diffuse.
    pub rim_strength: f32,
    /// Rim-light falloff exponent; higher = thinner rim.
    pub rim_exponent: f32,
}

impl ShadingParams {
    #[must_use]
    pub const fn new(bands: f32, ambient_floor: f32, rim_strength: f32, rim_exponent: f32) -> Self {
        Self {
            bands,
            ambient_floor,
            rim_strength,
            rim_exponent,
        }
    }

    const fn to_vec4(self) -> [f32; 4] {
        [
            self.bands,
            self.ambient_floor,
            self.rim_strength,
            self.rim_exponent,
        ]
    }
}

impl Default for ShadingParams {
    fn default() -> Self {
        Self::new(3.0, 0.4, 0.25, 2.5)
    }
}

/// Scene-level shading parameters: sun direction / color, ambient, fog.
///
/// Bound as a uniform at `@group(2)` on the scene pipeline. Milestone 2.6 sub-step 1
/// introduces the plumbing with the sun direction replacing the Phase 1 hard-coded
/// constant; fog and ambient become load-bearing in subsequent sub-steps (procedural
/// sky + height fog).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneParams {
    /// Unit vector pointing FROM the scene TOWARD the sun. `(0, 1, 0)` = sun overhead.
    pub sun_dir: Vec3,
    /// RGB sun-light color. `(1, 1, 1)` = neutral white.
    pub sun_color: Vec3,
    /// RGB ambient fill (reserved for sub-step 3).
    pub ambient: Vec3,
    /// RGB fog color (reserved for sub-step 3).
    pub fog_color: Vec3,
    /// Exponential fog density, per meter (reserved for sub-step 3).
    pub fog_density: f32,
    /// Wall-clock seconds since app start. Drives water Gerstner wave phase (and later
    /// any other time-animated shader). Non-deterministic w.r.t. the sim tick, which is
    /// intentional -- render is presentation-only.
    pub time_seconds: f32,
    /// Chop-intensity multiplier applied to water wave amplitudes in the
    /// shader. `0.0` = glassy, `1.0` = nominal (Phase 2 baseline), `2.0+` =
    /// heavy chop. App sets this to mirror `SimState.chop_intensity` so
    /// visual waves track physics chop (closes the 3.2.3c wiring gap).
    pub chop_intensity: f32,
}

impl SceneParams {
    fn to_uniform(self) -> SceneUniform {
        let d = self.sun_dir.try_normalize().unwrap_or(Vec3::Y);
        SceneUniform {
            sun_dir: [d.x, d.y, d.z, 0.0],
            sun_color: [self.sun_color.x, self.sun_color.y, self.sun_color.z, 0.0],
            ambient: [self.ambient.x, self.ambient.y, self.ambient.z, 0.0],
            fog: [
                self.fog_color.x,
                self.fog_color.y,
                self.fog_color.z,
                self.fog_density,
            ],
            time: [self.time_seconds, self.chop_intensity, 0.0, 0.0],
        }
    }
}

impl Default for SceneParams {
    /// Defaults reproduce the Phase 1 hard-coded light direction in the new
    /// toward-sun convention (the prior `light_dir = (0.45, -0.9, 0.3)` inverted).
    fn default() -> Self {
        Self {
            sun_dir: Vec3::new(-0.45, 0.9, -0.3),
            sun_color: Vec3::splat(1.0),
            ambient: Vec3::splat(0.35),
            fog_color: Vec3::new(0.55, 0.73, 0.90),
            // 0.00008 /m: ~4% fog at 500 m, ~8% at 1 km, ~55% at 10 km,
            // ~80% at 20 km. Open-visibility tuning for the Phase 3.6.3c.5b
            // streaming mesh (10 km half-extent ≈ 14 km diagonal). The
            // previous 0.0005/m was tuned for a 4 km test world and made
            // water read as sky past ~5 km — see `streaming_render_architecture`
            // memory + PROGRESS for the Option-2 tuning decision. Per-biome
            // tuning + performance-slider user control land in Phase 4.8.
            fog_density: 0.000_08,
            time_seconds: 0.0,
            // Default matches the prior Phase 2 baseline where SimState's
            // chop_intensity sat at 1.0 unconditionally.
            chop_intensity: 1.0,
        }
    }
}

/// Camera-follow snap pitch for the water mesh, meters. The water mesh's XZ origin
/// snaps to multiples of this so vertex positions stay phase-stable frame-to-frame
/// (avoids sub-meter shimmer). Picked smaller than typical wave wavelengths so the
/// snap is invisible.
const WATER_FOLLOW_SNAP_M: f32 = 4.0;

/// Embedded WGSL shader source. Exposed so tests can verify the binary actually
/// has the latest shader edits.
pub const WATER_WGSL_SOURCE: &str = include_str!("water.wgsl");

/// Cloud-noise WGSL source — Worley + PCG hash + helpers.
///
/// Exported so (a) the cloud raymarcher (Phase 3.6.3c.6 / 3.2x) can
/// `include_str!` this same file via this constant if a higher-level
/// shader composer lands, and (b) the offline `cloud_noise_wgsl_parses`
/// test can run naga validation without booting wgpu.
///
/// Bit-identical to `flowplan_world::noise::worley_2d` /
/// `worley_3d` per `docs/WEATHER.md` § Worley noise primitive — same
/// PCG hash, same rounding, same constants. Verified by GPU readback
/// in the bit-identity gate (Phase B4).
pub const CLOUD_NOISE_WGSL_SOURCE: &str = include_str!("cloud_noise.wgsl");

/// Volumetric cloud raymarcher WGSL source (Phase 3.6.3c.6 / 3.2x B2).
///
/// Empty raymarch skeleton — full-screen triangle pass that runs after
/// outline + before widgets, returns transparent in B2. B3 adds the
/// W2 weather-cell sampling + Perlin-Worley sub-cell detail. B4 adds
/// CPU-GPU bit-identity gate.
///
/// See `docs/WEATHER.md` § Cloud rendering technique for technique
/// decisions (pass placement, density storage, lighting model).
pub const VOLUMETRIC_WGSL_SOURCE: &str = include_str!("volumetric.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SceneUniform {
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    fog: [f32; 4],
    // x = seconds since app start; remaining lanes reserved.
    time: [f32; 4],
}

/// A wgpu-resident mesh: vertex buffer, index buffer, index count.
struct GpuMesh {
    vbo: wgpu::Buffer,
    ibo: wgpu::Buffer,
    index_count: u32,
}

impl GpuMesh {
    fn upload(device: &wgpu::Device, mesh: &CpuMesh) -> Self {
        let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-vbo", mesh.label)),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-ibo", mesh.label)),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vbo,
            ibo,
            index_count: mesh.indices.len() as u32,
        }
    }
}

/// A model that's drawn each frame with its own `ModelUniform` and bind group.
struct DrawModel {
    mesh: GpuMesh,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    shading: [f32; 4],
    tint: [f32; 4],
}

/// Top-level renderer.
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    /// Volumetric cloud raymarcher pipeline (Phase 3.6.3c.6 / 3.2x B2
    /// skeleton — returns transparent; B3+ adds density sampling).
    volumetric_pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,
    camera_buf: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    model_bgl: wgpu::BindGroupLayout,
    scene_bg: wgpu::BindGroup,
    scene_buf: wgpu::Buffer,
    outline_bgl: wgpu::BindGroupLayout,
    outline_bg: wgpu::BindGroup,
    outline_params_buf: wgpu::Buffer,
    terrain: DrawModel,
    /// Per-Voronoi-cell terrain chunks (Phase 3.6.3c.4). When non-
    /// empty, the render loop draws each chunk in turn instead of the
    /// single `terrain.mesh`. Cell-id keyed for cheap insert / evict
    /// from the streaming pipeline.
    terrain_chunks: std::collections::HashMap<u32, GpuMesh>,
    water: DrawModel,
    /// Per-Voronoi-cell water chunks (Phase 3.6.3c.5). Same pattern
    /// as `terrain_chunks` — when non-empty, render loop draws each
    /// instead of the single `water.mesh`.
    water_chunks: std::collections::HashMap<u32, GpuMesh>,
    /// When `false`, the water pass is skipped on render. Used by wheels-variant
    /// scenario playback so a y=0 flat-land terrain isn't hidden by a y=0 water
    /// mesh. Default `true` so free-flight and water-scenario playback retain
    /// the existing Gerstner surface.
    draw_water: bool,
    /// When `true`, the water mesh's model matrix translates to follow
    /// the camera each frame (old behavior for fixed-origin water mesh
    /// in scenarios). When `false`, the model matrix is identity — the
    /// water mesh is assumed to already have world-coordinate vertices
    /// (3.6.3c.5b streaming water). Default `true` to keep scenario
    /// playback unchanged; streaming setup in `game_app` flips it.
    water_follows_camera: bool,
    vehicle: DrawModel,
    /// When `false`, the vehicle draw call is skipped. Used by 3.6.3c.5a
    /// debug camera (aircraft pinned to camera position; don't render
    /// it from inside). Default `true`.
    draw_vehicle: bool,
    /// Phase 3.6.3c.5c.1: feature-instance renderer. Draws unit-box
    /// primitives at per-instance (position, rotation, scale, color)
    /// from a storage buffer. Populated by `game_app` from
    /// `BakedAssignments::instances_at_cell` when the active cell set
    /// changes.
    instances: InstanceRenderer,
    /// 2.4.2c: line-list pipeline used by the corner widgets and (later) HUD ladders
    /// and wireframe debug views.
    debug_lines: DebugLinesRenderer,
    /// Widget uniform for the bottom-right orientation gizmo.
    widget_camera_buf_br: wgpu::Buffer,
    widget_camera_bg_br: wgpu::BindGroup,
    /// Widget uniform for the bottom-left attitude indicator (2.4.4e).
    widget_camera_buf_bl: wgpu::Buffer,
    widget_camera_bg_bl: wgpu::BindGroup,
    // Kept for shader hot-reload pipeline rebuilds; feature-gated to avoid dead-code
    // warnings in release/default builds. Re-exposed without the gate when another
    // caller needs them.
    #[cfg(feature = "shader-hot-reload")]
    scene_pipeline_layout: wgpu::PipelineLayout,
    #[cfg(feature = "shader-hot-reload")]
    outline_pipeline_layout: wgpu::PipelineLayout,
    #[cfg(feature = "shader-hot-reload")]
    surface_format: wgpu::TextureFormat,
    #[cfg(feature = "shader-hot-reload")]
    watcher: Option<crate::hot_reload::Watcher>,
}

impl Renderer {
    /// Build the renderer.
    ///
    /// Takes the device/queue pair, the final surface color format (so the pipeline is
    /// configured correctly), the initial window size, and the two Phase 1 meshes.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        initial_size: (u32, u32),
        terrain_mesh: &CpuMesh,
        terrain_shading: ShadingParams,
        water_mesh: &CpuMesh,
        water_shading: ShadingParams,
        vehicle_mesh: &CpuMesh,
        vehicle_shading: ShadingParams,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-scene-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-outline-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("outline.wgsl").into()),
        });
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-sky-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()),
        });
        // Cloud raymarcher (3.6.3c.6 / 3.2x). Composed at shader-creation
        // time from cloud_noise.wgsl (Worley + PCG + helpers — also used
        // standalone for CPU-GPU bit-identity testing) followed by
        // volumetric.wgsl (cloud-density model + raymarch + entry points).
        // Naming: the noise primitives must be declared BEFORE the
        // volumetric module that calls them.
        let volumetric_source = format!(
            "{}\n{}",
            include_str!("cloud_noise.wgsl"),
            include_str!("volumetric.wgsl")
        );
        let volumetric_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-volumetric-shader"),
            source: wgpu::ShaderSource::Wgsl(volumetric_source.into()),
        });
        let water_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-water-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("water.wgsl").into()),
        });

        // Bind group layouts.
        // Camera uniform is read by vertex (MVP) AND fragment (rim-light view dir).
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        core::mem::size_of::<CameraUniform>() as u64
                    ),
                },
                count: None,
            }],
        });
        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        core::mem::size_of::<ModelUniform>() as u64
                    ),
                },
                count: None,
            }],
        });

        // Scene-level uniform (sun / ambient / fog / time). Read by the scene fragment
        // shader AND the water vertex shader (time drives Gerstner wave phase).
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        core::mem::size_of::<SceneUniform>() as u64
                    ),
                },
                count: None,
            }],
        });

        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("phase2-scene-pipeline-layout"),
                bind_group_layouts: &[&camera_bgl, &model_bgl, &scene_bgl],
                push_constant_ranges: &[],
            });

        let pipeline =
            build_scene_pipeline(device, &scene_pipeline_layout, &shader, surface_format);
        // Water reuses the camera + model + scene bind group layouts but has its own
        // pipeline with the Gerstner vertex shader and the water fragment shader.
        let water_pipeline = build_water_pipeline(
            device,
            &scene_pipeline_layout,
            &water_shader,
            surface_format,
        );

        let depth_view = create_depth_view(device, initial_size);

        // Outline bind-group layout: depth texture (sampled as depth) + params uniform.
        let outline_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("outline-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            core::mem::size_of::<OutlineUniform>() as u64,
                        ),
                    },
                    count: None,
                },
            ],
        });
        let outline_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("outline-pipeline-layout"),
                bind_group_layouts: &[&outline_bgl],
                push_constant_ranges: &[],
            });
        let outline_pipeline = build_outline_pipeline(
            device,
            &outline_pipeline_layout,
            &outline_shader,
            surface_format,
        );

        // Sky pipeline layout uses the existing camera + scene bind group layouts. The
        // sky shader maps them as: @group(0) = camera, @group(1) = scene.
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("phase2-sky-pipeline-layout"),
            bind_group_layouts: &[&camera_bgl, &scene_bgl],
            push_constant_ranges: &[],
        });
        let sky_pipeline =
            build_sky_pipeline(device, &sky_pipeline_layout, &sky_shader, surface_format);

        // Volumetric cloud pipeline layout shares the camera + scene bind groups with
        // sky. B3+ adds a third group for scene-depth read-only access (ray
        // termination at terrain). The skeleton (B2) doesn't read depth; uniform
        // bindings are sufficient.
        let volumetric_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flowplan-volumetric-pipeline-layout"),
                bind_group_layouts: &[&camera_bgl, &scene_bgl],
                push_constant_ranges: &[],
            });
        let volumetric_pipeline = build_volumetric_pipeline(
            device,
            &volumetric_pipeline_layout,
            &volumetric_shader,
            surface_format,
        );

        let outline_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("outline-params-ubo"),
            size: core::mem::size_of::<OutlineUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let outline_bg =
            build_outline_bind_group(device, &outline_bgl, &depth_view, &outline_params_buf);

        // Camera uniform.
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera-ubo"),
            size: core::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        // Scene uniform buffer + bind group; primed with defaults so frame 0 is valid
        // even if the app never calls `set_scene`.
        let scene_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene-ubo"),
            size: core::mem::size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene-bg"),
            layout: &scene_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_buf.as_entire_binding(),
            }],
        });
        queue.write_buffer(
            &scene_buf,
            0,
            bytemuck::bytes_of(&SceneParams::default().to_uniform()),
        );

        // Terrain draw model (identity transform, light tint).
        let terrain = DrawModel::new(
            device,
            &model_bgl,
            terrain_mesh,
            Mat4::IDENTITY,
            Vec4::new(1.0, 1.0, 1.0, 1.0),
            terrain_shading,
        );

        // Water draw model (identity transform; vertex colors carry the water hue).
        let water = DrawModel::new(
            device,
            &model_bgl,
            water_mesh,
            Mat4::IDENTITY,
            Vec4::new(1.0, 1.0, 1.0, 1.0),
            water_shading,
        );

        // Vehicle draw model. Initial transform is identity; `render` overwrites each frame.
        let vehicle = DrawModel::new(
            device,
            &model_bgl,
            vehicle_mesh,
            Mat4::IDENTITY,
            Vec4::new(1.0, 1.0, 1.0, 1.0),
            vehicle_shading,
        );

        // Prime camera uniform to a sensible default so the first frame does not stutter.
        let default_view_proj =
            perspective_view_proj(Quat::IDENTITY, 60_f32.to_radians(), aspect(initial_size));
        queue.write_buffer(
            &camera_buf,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: default_view_proj.to_cols_array_2d(),
                view_pos: [0.0, 5.0, 15.0, 0.0],
                inverse_view_proj: default_view_proj.inverse().to_cols_array_2d(),
            }),
        );

        #[cfg(feature = "shader-hot-reload")]
        let watcher = match crate::hot_reload::Watcher::new() {
            Ok(w) => Some(w),
            Err(e) => {
                // Dev-only feature; failing to spawn the watcher is not fatal.
                eprintln!("flowplan-render: shader-hot-reload disabled: {e}");
                None
            },
        };

        // 3.6.3c.5c.1 + 5c.2.a: feature-instance renderer. One mesh
        // per primitive kind (box / cone / cylinder / sphere). Shares
        // the camera + scene bind groups with the main scene pipeline.
        // Instance buffers start empty; `game_app` uploads per-kind
        // when the active cell set resolves.
        let instance_meshes = InstanceRenderer::build_primitive_meshes();
        let instance_mesh_refs: Vec<&CpuMesh> = instance_meshes.iter().collect();
        let instances = InstanceRenderer::new(
            device,
            &camera_bgl,
            &scene_bgl,
            surface_format,
            &instance_mesh_refs,
        );

        // Two slots: slot 0 = bottom-right orientation gizmo, slot 1 = bottom-left
        // attitude indicator. Each has its own VBO pair so queue writes between
        // render passes in the same command buffer don't clobber each other.
        let debug_lines = DebugLinesRenderer::new(device, &camera_bgl, surface_format, 2);

        // Two widget uniforms: one per corner-anchored debug widget. Each holds a
        // screen-space ortho view-proj recomputed per frame from viewport size.
        // `view_pos` and `inverse_view_proj` are unused by the line shader; left zero.
        let widget_camera_buf_br = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("widget-camera-br-ubo"),
            size: core::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let widget_camera_bg_br = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("widget-camera-br-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: widget_camera_buf_br.as_entire_binding(),
            }],
        });
        let widget_camera_buf_bl = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("widget-camera-bl-ubo"),
            size: core::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let widget_camera_bg_bl = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("widget-camera-bl-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: widget_camera_buf_bl.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            water_pipeline,
            outline_pipeline,
            sky_pipeline,
            volumetric_pipeline,
            depth_view,
            camera_buf,
            camera_bg,
            model_bgl,
            scene_bg,
            scene_buf,
            outline_bgl,
            outline_bg,
            outline_params_buf,
            terrain,
            terrain_chunks: std::collections::HashMap::new(),
            water,
            water_chunks: std::collections::HashMap::new(),
            draw_water: true,
            water_follows_camera: true,
            draw_vehicle: true,
            vehicle,
            instances,
            debug_lines,
            widget_camera_buf_br,
            widget_camera_bg_br,
            widget_camera_buf_bl,
            widget_camera_bg_bl,
            #[cfg(feature = "shader-hot-reload")]
            scene_pipeline_layout,
            #[cfg(feature = "shader-hot-reload")]
            outline_pipeline_layout,
            #[cfg(feature = "shader-hot-reload")]
            surface_format,
            #[cfg(feature = "shader-hot-reload")]
            watcher,
        }
    }

    /// Check the shader-reload flag and rebuild pipelines if a watched file changed.
    /// No-op in release builds (feature `shader-hot-reload` disabled by default).
    #[cfg(feature = "shader-hot-reload")]
    pub fn maybe_reload_shaders(&mut self, device: &wgpu::Device) {
        let Some(watcher) = self.watcher.as_ref() else {
            return;
        };
        if !watcher.take_pending() {
            return;
        }
        let scene_src = match crate::hot_reload::load_source("shader.wgsl") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("flowplan-render: scene shader read failed: {e}");
                return;
            },
        };
        let outline_src = match crate::hot_reload::load_source("outline.wgsl") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("flowplan-render: outline shader read failed: {e}");
                return;
            },
        };
        let scene_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-scene-shader-reload"),
            source: wgpu::ShaderSource::Wgsl(scene_src.into()),
        });
        let outline_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-outline-shader-reload"),
            source: wgpu::ShaderSource::Wgsl(outline_src.into()),
        });
        self.pipeline = build_scene_pipeline(
            device,
            &self.scene_pipeline_layout,
            &scene_mod,
            self.surface_format,
        );
        self.outline_pipeline = build_outline_pipeline(
            device,
            &self.outline_pipeline_layout,
            &outline_mod,
            self.surface_format,
        );
        eprintln!("flowplan-render: shaders reloaded");
    }

    /// Write scene-level parameters (sun direction / color, ambient, fog) into the
    /// scene uniform buffer. Called each frame before `render`.
    /// Toggle whether the water mesh is rendered. Default `true`. Set to
    /// `false` for wheels-variant scenarios on flat-land terrain, where the
    /// y=0 water surface would z-fight the y=0 terrain.
    /// Replace the terrain `GpuMesh` from a freshly-built `CpuMesh`.
    ///
    /// Used by the streaming-render layer (Phase 3.6.3c.5b) when the
    /// active camera moves enough to trigger a mesh rebuild. Old VBO /
    /// IBO are dropped (wgpu handles GPU lifetime — buffers are kept
    /// alive until current GPU work finishes).
    pub fn update_terrain_mesh(&mut self, device: &wgpu::Device, mesh: &flowplan_assets::CpuMesh) {
        self.terrain.mesh = GpuMesh::upload(device, mesh);
    }

    /// Insert or replace a per-cell terrain chunk (Phase 3.6.3c.4).
    /// When chunks are present, the render pass draws each chunk
    /// instead of the single `terrain.mesh`. Caller manages the
    /// active set via `update_terrain_chunk` + `evict_terrain_chunk`.
    pub fn update_terrain_chunk(
        &mut self,
        device: &wgpu::Device,
        cell_id: u32,
        mesh: &flowplan_assets::CpuMesh,
    ) {
        // Skip empty meshes — chunks where the cell footprint produced
        // zero in-cell vertices (rare; e.g. degenerate Voronoi cell
        // entirely outside the bbox). Empty VBO is undefined-behavior
        // bait; just skip the upload.
        if mesh.vertices.is_empty() || mesh.indices.is_empty() {
            return;
        }
        self.terrain_chunks
            .insert(cell_id, GpuMesh::upload(device, mesh));
    }

    /// Drop a terrain chunk (cell exited the streaming radius).
    pub fn evict_terrain_chunk(&mut self, cell_id: u32) {
        self.terrain_chunks.remove(&cell_id);
    }

    /// True if the chunked-terrain path is active (>=1 chunk resident).
    /// When false, the render loop falls back to the single
    /// `terrain.mesh` (legacy / scenario / fast-iteration path).
    #[must_use]
    pub fn has_terrain_chunks(&self) -> bool {
        !self.terrain_chunks.is_empty()
    }

    /// Replace the water `GpuMesh` from a freshly-built `CpuMesh`.
    ///
    /// Same pattern as [`Self::update_terrain_mesh`] for water streaming.
    pub fn update_water_mesh(&mut self, device: &wgpu::Device, mesh: &flowplan_assets::CpuMesh) {
        self.water.mesh = GpuMesh::upload(device, mesh);
    }

    /// Insert or replace a per-cell water chunk (Phase 3.6.3c.5).
    /// Mirror of `update_terrain_chunk` for water.
    pub fn update_water_chunk(
        &mut self,
        device: &wgpu::Device,
        cell_id: u32,
        mesh: &flowplan_assets::CpuMesh,
    ) {
        if mesh.vertices.is_empty() || mesh.indices.is_empty() {
            return;
        }
        self.water_chunks
            .insert(cell_id, GpuMesh::upload(device, mesh));
    }

    /// Drop a water chunk (cell exited the streaming radius).
    pub fn evict_water_chunk(&mut self, cell_id: u32) {
        self.water_chunks.remove(&cell_id);
    }

    /// True if the chunked-water path is active (>=1 chunk resident).
    #[must_use]
    pub fn has_water_chunks(&self) -> bool {
        !self.water_chunks.is_empty()
    }

    pub fn set_water_visible(&mut self, visible: bool) {
        self.draw_water = visible;
    }

    /// Toggle rendering of the vehicle mesh.
    ///
    /// Used by 3.6.3c.5a — when debug camera is active, the aircraft is
    /// pinned to the camera position (so the user isn't looking at the
    /// inside of their own aircraft mesh). Defaults to true.
    pub fn set_vehicle_visible(&mut self, visible: bool) {
        self.draw_vehicle = visible;
    }

    /// When `false`, the water mesh's model matrix stays identity —
    /// appropriate when the mesh vertices are in world coordinates
    /// (3.6.3c.5b streaming water). When `true` (default), the mesh
    /// translates to follow the camera each frame (old fixed-origin
    /// water behavior for scenario playback).
    pub fn set_water_follows_camera(&mut self, follows: bool) {
        self.water_follows_camera = follows;
    }

    /// Phase 3.6.3c.5c.1 + 5c.2.a: replace the active-set instance
    /// list for a single primitive kind. Caller groups instances by
    /// kind + calls this once per kind per rebuild. Empty slice clears
    /// the kind's draw (no-op on record).
    pub fn upload_instances_for(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        kind: flowplan_world::feature_instances::PrimitiveKind,
        data: &[InstanceGpuData],
    ) {
        self.instances.upload_kind(device, queue, kind, data);
    }

    /// Live instance count across all primitive kinds. Debug HUD +
    /// telemetry consumer.
    #[must_use]
    pub fn instance_count(&self) -> u32 {
        self.instances.instance_count()
    }

    pub fn set_scene(&self, queue: &wgpu::Queue, params: &SceneParams) {
        queue.write_buffer(&self.scene_buf, 0, bytemuck::bytes_of(&params.to_uniform()));
    }

    /// Handle a window resize.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        self.depth_view = create_depth_view(device, size);
        // Outline bind group references the depth view by handle; rebuild it so the
        // outline pass reads the fresh depth texture rather than the dropped one.
        self.outline_bg = build_outline_bind_group(
            device,
            &self.outline_bgl,
            &self.depth_view,
            &self.outline_params_buf,
        );
    }

    /// Intentional read accessor; render crate may grow multiple model-bind-group layouts.
    pub fn model_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.model_bgl
    }

    /// Render one frame.
    ///
    /// Interpolates between `snapshot_prev` and `snapshot_curr` by `alpha in [0, 1]`.
    /// Writes uniforms, begins a render pass with the depth buffer, draws terrain and
    /// vehicle, and ends the pass. Caller submits the encoder.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        size: (u32, u32),
        snapshot_prev: &SimSnapshot,
        snapshot_curr: &SimSnapshot,
        alpha: f32,
        clear_color: wgpu::Color,
        orientation_widget: Option<&DebugLineBatch>,
        attitude_widget: Option<&DebugLineBatch>,
    ) {
        let alpha = alpha.clamp(0.0, 1.0);

        // Interpolate camera.
        let cam_pos = snapshot_prev
            .camera
            .position
            .lerp(snapshot_curr.camera.position, alpha);
        let cam_orient = snapshot_prev
            .camera
            .orientation
            .slerp(snapshot_curr.camera.orientation, alpha);
        // FOV is typically constant across one tick; just take current.
        let fov = snapshot_curr.camera.fov_y_radians;
        let view_proj = perspective_view_proj(cam_orient, fov, aspect(size));
        queue.write_buffer(
            &self.camera_buf,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: view_proj.to_cols_array_2d(),
                view_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 0.0],
                inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            }),
        );

        // Interpolate vehicle.
        let veh_pos = snapshot_prev
            .vehicle
            .position
            .lerp(snapshot_curr.vehicle.position, alpha);
        let veh_orient = snapshot_prev
            .vehicle
            .orientation
            .slerp(snapshot_curr.vehicle.orientation, alpha);
        // Vehicle renders at authored scale: the placeholder cube ships sized correctly
        // from the app; glTF-imported meshes are in real-world meters.
        let model_mat = Mat4::from_scale_rotation_translation(Vec3::ONE, veh_orient, veh_pos);
        let normal_mat = Mat4::from_quat(veh_orient);
        let vehicle_uniform = ModelUniform {
            model: model_mat.to_cols_array_2d(),
            normal_mat: normal_mat.to_cols_array_2d(),
            tint: self.vehicle.tint,
            shading: self.vehicle.shading,
        };
        queue.write_buffer(
            &self.vehicle.uniform_buf,
            0,
            bytemuck::bytes_of(&vehicle_uniform),
        );

        // Water mesh model matrix.
        //
        // Scenario / fixed-mesh mode (`water_follows_camera = true`): the
        // mesh has local-origin vertices; translate to follow camera so
        // the aircraft stays inside the tessellated water area. Snap to a
        // few-meter pitch for shimmer-free frame-to-frame alignment.
        //
        // Streaming-water mode (3.6.3c.5b, `water_follows_camera = false`):
        // the mesh's vertices are already in world coordinates (rebuilt
        // when the camera crosses the recenter threshold). Identity
        // matrix — any translation here would double-apply and make the
        // water plane visibly drift with the camera.
        let water_model = if self.water_follows_camera {
            let snap_x = (cam_pos.x / WATER_FOLLOW_SNAP_M).round() * WATER_FOLLOW_SNAP_M;
            let snap_z = (cam_pos.z / WATER_FOLLOW_SNAP_M).round() * WATER_FOLLOW_SNAP_M;
            Mat4::from_translation(Vec3::new(snap_x, 0.0, snap_z))
        } else {
            Mat4::IDENTITY
        };
        let water_uniform = ModelUniform {
            model: water_model.to_cols_array_2d(),
            normal_mat: Mat4::IDENTITY.to_cols_array_2d(),
            tint: self.water.tint,
            shading: self.water.shading,
        };
        queue.write_buffer(
            &self.water.uniform_buf,
            0,
            bytemuck::bytes_of(&water_uniform),
        );

        // Outline params: viewport size + tunables, written every frame so a resize is
        // reflected immediately without a dedicated resize hook for the params buffer.
        let outline_params = OutlineUniform {
            viewport_and_tunables: [
                size.0 as f32,
                size.1 as f32,
                OUTLINE_THRESHOLD,
                OUTLINE_STRENGTH,
            ],
        };
        queue.write_buffer(
            &self.outline_params_buf,
            0,
            bytemuck::bytes_of(&outline_params),
        );

        // Sky pass. Renders first, filling the target with an analytic sky gradient +
        // sun disc. The scene pass then draws on top with `LoadOp::Load`.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("phase2-sky-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.sky_pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.set_bind_group(1, &self.scene_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Scene pass.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("phase2-scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.set_bind_group(2, &self.scene_bg, &[]);

            // Draw terrain. Two modes:
            //
            // - **Chunked** (Phase 3.6.3c.4): per-Voronoi-cell meshes.
            //   The terrain.bind_group's identity model matrix applies
            //   to every chunk (chunks are in world coords), so we
            //   bind it once and swap VBO/IBO per chunk. ~440 draw
            //   calls at 22 km streaming radius — well within budget.
            // - **Single** (legacy): one terrain.mesh, used by
            //   scenario playback + fast-iteration mode.
            pass.set_bind_group(1, &self.terrain.bind_group, &[]);
            if self.terrain_chunks.is_empty() {
                pass.set_vertex_buffer(0, self.terrain.mesh.vbo.slice(..));
                pass.set_index_buffer(self.terrain.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.terrain.mesh.index_count, 0, 0..1);
            } else {
                for chunk in self.terrain_chunks.values() {
                    pass.set_vertex_buffer(0, chunk.vbo.slice(..));
                    pass.set_index_buffer(chunk.ibo.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..chunk.index_count, 0, 0..1);
                }
            }

            // Draw water with its own pipeline (Gerstner vertex displacement + water
            // fragment). Camera + model + scene bind groups are layout-compatible, so
            // we just swap the pipeline. Skipped entirely for wheels-variant scenes
            // where the terrain sits at y=0 (no visible body of water).
            if self.draw_water {
                pass.set_pipeline(&self.water_pipeline);
                pass.set_bind_group(0, &self.camera_bg, &[]);
                pass.set_bind_group(2, &self.scene_bg, &[]);
                pass.set_bind_group(1, &self.water.bind_group, &[]);
                if self.water_chunks.is_empty() {
                    // Legacy single-mesh path — scenarios + fast-iteration.
                    pass.set_vertex_buffer(0, self.water.mesh.vbo.slice(..));
                    pass.set_index_buffer(self.water.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..self.water.mesh.index_count, 0, 0..1);
                } else {
                    // Chunked path — iterate active per-cell water meshes.
                    for chunk in self.water_chunks.values() {
                        pass.set_vertex_buffer(0, chunk.vbo.slice(..));
                        pass.set_index_buffer(chunk.ibo.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..chunk.index_count, 0, 0..1);
                    }
                }
            }

            // Back to the scene pipeline for the vehicle. Skipped when
            // 3.6.3c.5a debug camera is active (aircraft pinned to
            // camera position; rendering would put the user inside the
            // vehicle mesh).
            if self.draw_vehicle {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.camera_bg, &[]);
                pass.set_bind_group(2, &self.scene_bg, &[]);
                pass.set_bind_group(1, &self.vehicle.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vehicle.mesh.vbo.slice(..));
                pass.set_index_buffer(self.vehicle.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.vehicle.mesh.index_count, 0, 0..1);
            }

            // 3.6.3c.5c.1: feature instances. Own pipeline (storage-
            // buffer instanced draw). No-op when count is 0.
            self.instances
                .record(&mut pass, &self.camera_bg, &self.scene_bg);
        }

        // Outline pass: full-screen triangle, sobel on the just-written depth buffer,
        // alpha-blended black on top of the scene.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("phase2-outline-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.outline_pipeline);
            pass.set_bind_group(0, &self.outline_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Volumetric cloud pass (Phase 3.6.3c.6 / 3.2x B2 — skeleton).
        // Runs AFTER outline so cel-shaded silhouettes apply to opaque
        // geometry first; clouds then composite over outlined scene
        // (where cloud is dense enough, it occludes both terrain and
        // outline — correct atmospheric layering). B3 will add the
        // depth attachment for ray termination at terrain. B2's empty
        // raymarch returns alpha = 0, so this pass is a visual no-op
        // until then.
        //
        // Pass ordering pinned per docs/WEATHER.md § Render pass
        // placement: sky → scene → outline → cloud → widgets.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("flowplan-volumetric-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.volumetric_pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.set_bind_group(1, &self.scene_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Widget passes (Phase 2.4.2c orientation gizmo + 2.4.4e attitude indicator).
        // Each widget has its own ortho view-proj and screen-space rectangle. Drawn
        // last so the widgets always overlay; no depth attachment. Scissor rect
        // clamps rendering to each widget's box so geometry that runs past widget-
        // local `[-1, 1]` doesn't bleed into the rest of the screen (the attitude
        // indicator's horizon can extend past bounds at extreme pitch).
        for (slot, batch_opt, anchor, camera_buf, camera_bg, label) in [
            (
                0usize,
                orientation_widget,
                WidgetAnchor::BottomRight,
                &self.widget_camera_buf_br,
                &self.widget_camera_bg_br,
                "orientation-widget-pass",
            ),
            (
                1,
                attitude_widget,
                WidgetAnchor::BottomLeft,
                &self.widget_camera_buf_bl,
                &self.widget_camera_bg_bl,
                "attitude-widget-pass",
            ),
        ] {
            let Some(batch) = batch_opt else { continue };
            if batch.is_empty() {
                continue;
            }
            let widget_vp = widget_view_proj(size, anchor);
            queue.write_buffer(
                camera_buf,
                0,
                bytemuck::bytes_of(&CameraUniform {
                    view_proj: widget_vp.to_cols_array_2d(),
                    view_pos: [0.0; 4],
                    inverse_view_proj: [[0.0; 4]; 4],
                }),
            );
            self.debug_lines.upload(device, queue, slot, batch);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let (sx, sy, sw, sh) = widget_scissor_rect(size, anchor);
            if sw > 0 && sh > 0 {
                pass.set_scissor_rect(sx, sy, sw, sh);
            }
            pass.set_bind_group(0, camera_bg, &[]);
            self.debug_lines.render(&mut pass, slot, batch);
        }
    }
}

/// Widget size in pixels (square).
const WIDGET_PX: f32 = 180.0;
/// Margin from the viewport edges (pixels).
const WIDGET_MARGIN_PX: f32 = 20.0;

/// Anchor corner for a debug widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetAnchor {
    BottomRight,
    BottomLeft,
}

/// Screen-space orthographic view-proj for a corner-anchored debug widget.
///
/// Maps widget-local `[-1, 1]³` to a `WIDGET_PX × WIDGET_PX` square anchored
/// `WIDGET_MARGIN_PX` from the chosen corner. Z range `[-1, 1]` is mapped to NDC
/// `[0, 1]`; the line pipeline currently has depth disabled so Z only matters for
/// future depth-aware consumers.
fn widget_view_proj(viewport: (u32, u32), anchor: WidgetAnchor) -> Mat4 {
    let vw = viewport.0.max(1) as f32;
    let vh = viewport.1.max(1) as f32;
    let sx = WIDGET_PX / vw;
    let sy = WIDGET_PX / vh;
    // In NDC, +X is right and +Y is up (wgpu).
    let cx_right = 1.0 - (WIDGET_PX + 2.0 * WIDGET_MARGIN_PX) / vw;
    let cx_left = -1.0 + (WIDGET_PX + 2.0 * WIDGET_MARGIN_PX) / vw;
    let cy_bottom = -1.0 + (WIDGET_PX + 2.0 * WIDGET_MARGIN_PX) / vh;
    let (cx, cy) = match anchor {
        WidgetAnchor::BottomRight => (cx_right, cy_bottom),
        WidgetAnchor::BottomLeft => (cx_left, cy_bottom),
    };
    Mat4::from_cols(
        Vec4::new(sx, 0.0, 0.0, 0.0),
        Vec4::new(0.0, sy, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.5, 0.0),
        Vec4::new(cx, cy, 0.5, 1.0),
    )
}

/// Scissor rectangle in pixels (x, y, width, height) for a widget. `x, y` is the
/// top-left corner in wgpu's top-left-origin convention.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]
fn widget_scissor_rect(viewport: (u32, u32), anchor: WidgetAnchor) -> (u32, u32, u32, u32) {
    let vw = viewport.0.max(1) as f32;
    let vh = viewport.1.max(1) as f32;
    let size = WIDGET_PX;
    let margin = WIDGET_MARGIN_PX;
    let (x_px, y_px) = match anchor {
        WidgetAnchor::BottomRight => (vw - size - margin, vh - size - margin),
        WidgetAnchor::BottomLeft => (margin, vh - size - margin),
    };
    let x = x_px.max(0.0) as u32;
    let y = y_px.max(0.0) as u32;
    let w = (size as u32).min(viewport.0.saturating_sub(x));
    let h = (size as u32).min(viewport.1.saturating_sub(y));
    (x, y, w, h)
}

impl DrawModel {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        mesh: &CpuMesh,
        initial_model: Mat4,
        tint: Vec4,
        shading: ShadingParams,
    ) -> Self {
        let mesh_gpu = GpuMesh::upload(device, mesh);
        let tint_arr = [tint.x, tint.y, tint.z, tint.w];
        let shading_arr = shading.to_vec4();
        let uniform = ModelUniform {
            model: initial_model.to_cols_array_2d(),
            normal_mat: initial_model.to_cols_array_2d(),
            tint: tint_arr,
            shading: shading_arr,
        };
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-model-ubo", mesh.label)),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{}-model-bg", mesh.label)),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        Self {
            mesh: mesh_gpu,
            uniform_buf,
            bind_group,
            shading: shading_arr,
            tint: tint_arr,
        }
    }
}

fn build_scene_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let vertex_attrs = wgpu::vertex_attr_array![
        0 => Float32x3, // position
        1 => Float32x3, // normal
        2 => Float32x2, // uv
        3 => Float32x4, // color
    ];
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: core::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &vertex_attrs,
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("phase2-scene-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &vertex_buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            // Culling stays off for now. Geometry winding checks out on the exterior
            // views used by the snapshot tests, but the runtime chase camera and the
            // first-person pose dip inside the vehicle mesh's bounding volume, which
            // makes the interior surfaces disappear and reads as "see through the
            // plane" to the player. Re-enable once Milestone 2.4+ introduces proper
            // chase-camera clearance and a cockpit model. Tracked as revisit on 2.3.4.
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn build_outline_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("phase2-outline-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_outline",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_outline",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        // Outline is a full-screen post pass; no depth interaction.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn build_water_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let vertex_attrs = wgpu::vertex_attr_array![
        0 => Float32x3, // position
        1 => Float32x3, // normal
        2 => Float32x2, // uv
        3 => Float32x4, // color
    ];
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: core::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &vertex_attrs,
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("phase2-water-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_water",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &vertex_buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_water",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn build_sky_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("phase2-sky-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_sky",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_sky",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                // Sky fully overwrites the cleared target; no blend needed.
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        // Sky runs without a depth attachment (full-screen, before scene).
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

/// Build the volumetric cloud raymarcher pipeline (Phase 3.6.3c.6 / 3.2x B2).
///
/// Full-screen triangle pass running AFTER outline + BEFORE widgets. Composites
/// alpha-blended over the rendered scene. B3 adds depth attachment for
/// scene-depth read-only ray termination; B2 skeleton uses no depth.
///
/// Per `docs/WEATHER.md` § Cloud rendering technique → Composite + cel-shading:
/// banded alpha quantization happens shader-side; pipeline blend is standard
/// pre-multiplied src-over for the alpha-composited cloud color over scene.
fn build_volumetric_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("flowplan-volumetric-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_volumetric",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_volumetric",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                // Cloud composites src-alpha over scene color. With B2's
                // alpha = 0 output, this is a no-op — the scene below shows
                // through unchanged.
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        // No depth attachment in B2 skeleton. B3 adds depth read-only for ray
        // termination at terrain.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn create_depth_view(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-buffer"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        // `TEXTURE_BINDING` is required so the outline pass can sample depth after the
        // scene pass writes it. The two passes are non-overlapping within the encoder.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    tex.create_view(&wgpu::TextureViewDescriptor::default())
}

fn build_outline_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    depth_view: &wgpu::TextureView,
    params_buf: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("outline-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params_buf.as_entire_binding(),
            },
        ],
    })
}

fn aspect((w, h): (u32, u32)) -> f32 {
    if h == 0 {
        1.0
    } else {
        w as f32 / h as f32
    }
}

/// Camera-relative view-projection (Phase 3.6.3c.5d).
///
/// Returns `proj * view_rot_only` — the view matrix has no translation
/// component. Vertex shaders must subtract `camera.view_pos` from
/// world-space vertex positions before applying this matrix; that
/// subtraction happens at similar magnitudes so the result is small
/// and the subsequent matmul stays precise.
///
/// Without this split, flying the camera hundreds of km from origin
/// produces visible z-fighting / vertex jitter / artifacting because
/// the view matrix's translation column contains values in the millions
/// and f32 precision is lost during `proj * view` intermediate terms.
/// See PROGRESS.md Phase 3.6.3c.5d for the incident + fix.
fn perspective_view_proj(orient: Quat, fov_y: f32, aspect: f32) -> Mat4 {
    let forward = orient * Vec3::NEG_Z;
    let up = orient * Vec3::Y;
    // look_to_rh with origin=0 gives a pure rotation matrix (no
    // translation). The actual camera position is delivered to shaders
    // via `CameraUniform.view_pos`; vertex shaders subtract it from
    // world positions before applying this matrix.
    let view = Mat4::look_to_rh(Vec3::ZERO, forward, up);
    let proj = Mat4::perspective_rh(fov_y, aspect, NEAR, FAR);
    proj * view
}

#[cfg(test)]
mod tests {
    // The renderer requires a live wgpu device; full tests live under tests/hardware per
    // TEST_PLAN.md. These unit tests exercise only pure helpers.

    use super::*;

    #[test]
    fn aspect_handles_zero_height() {
        assert!((aspect((1920, 0)) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn aspect_is_width_over_height() {
        assert!((aspect((1920, 1080)) - (1920.0 / 1080.0)).abs() < 1e-6);
    }

    #[test]
    fn view_proj_is_finite() {
        let m = perspective_view_proj(Quat::IDENTITY, 60_f32.to_radians(), 16.0 / 9.0);
        for col in m.to_cols_array() {
            assert!(col.is_finite());
        }
    }

    #[test]
    fn view_proj_has_no_translation_column() {
        // Camera-relative rendering (Phase 3.6.3c.5d): the view-proj
        // matrix must be rotation + projection only. Translation is
        // applied CPU-side via `CameraUniform.view_pos`; vertex shaders
        // subtract it before the matmul. Regression guard — if this
        // assertion fails, review where translation re-entered.
        let m = perspective_view_proj(Quat::IDENTITY, 60_f32.to_radians(), 16.0 / 9.0);
        // With camera at origin and identity orientation, a point at
        // (10, 0, -1) should project to NDC at approximately the same
        // clip-space coord regardless of "imagined" camera position.
        let cols = m.to_cols_array_2d();
        // The translation column is column 3 (last). For a rotation-
        // only + proj matrix, col[3] should have only perspective-divide
        // components (zeros in x/y rows, z-row carries (2*far*near)/(near-far)).
        assert!(cols[3][0].abs() < 1e-3, "col3.x nonzero: {}", cols[3][0]);
        assert!(cols[3][1].abs() < 1e-3, "col3.y nonzero: {}", cols[3][1]);
    }
}
