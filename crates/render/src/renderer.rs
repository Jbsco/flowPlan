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

/// Depth format for the Phase 1 forward pass.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Perspective near-plane distance (meters).
const NEAR: f32 = 0.1;
/// Perspective far-plane distance (meters).
const FAR: f32 = 10_000.0;

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
            time: [self.time_seconds, 0.0, 0.0, 0.0],
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
            // 0.0005 /m: ~5% fog at 100 m, ~39% at 1 km, ~63% at 2 km. Tuned for the
            // mountain-scale terrain (±375 m peaks) so distant ridges read through
            // atmospheric haze without disappearing. Per-biome tuning lands later.
            fog_density: 0.0005,
            time_seconds: 0.0,
        }
    }
}

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
    water: DrawModel,
    vehicle: DrawModel,
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
        let default_view_proj = perspective_view_proj(
            Vec3::new(0.0, 5.0, 15.0),
            Quat::IDENTITY,
            60_f32.to_radians(),
            aspect(initial_size),
        );
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

        Self {
            pipeline,
            water_pipeline,
            outline_pipeline,
            sky_pipeline,
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
            water,
            vehicle,
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
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        size: (u32, u32),
        snapshot_prev: &SimSnapshot,
        snapshot_curr: &SimSnapshot,
        alpha: f32,
        clear_color: wgpu::Color,
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
        let view_proj = perspective_view_proj(cam_pos, cam_orient, fov, aspect(size));
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

            // Draw terrain.
            pass.set_bind_group(1, &self.terrain.bind_group, &[]);
            pass.set_vertex_buffer(0, self.terrain.mesh.vbo.slice(..));
            pass.set_index_buffer(self.terrain.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.terrain.mesh.index_count, 0, 0..1);

            // Draw water with its own pipeline (Gerstner vertex displacement + water
            // fragment). Camera + model + scene bind groups are layout-compatible, so
            // we just swap the pipeline.
            pass.set_pipeline(&self.water_pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.set_bind_group(2, &self.scene_bg, &[]);
            pass.set_bind_group(1, &self.water.bind_group, &[]);
            pass.set_vertex_buffer(0, self.water.mesh.vbo.slice(..));
            pass.set_index_buffer(self.water.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.water.mesh.index_count, 0, 0..1);

            // Back to the scene pipeline for the vehicle.
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.set_bind_group(2, &self.scene_bg, &[]);
            pass.set_bind_group(1, &self.vehicle.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vehicle.mesh.vbo.slice(..));
            pass.set_index_buffer(self.vehicle.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.vehicle.mesh.index_count, 0, 0..1);
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
    }
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

fn perspective_view_proj(pos: Vec3, orient: Quat, fov_y: f32, aspect: f32) -> Mat4 {
    let forward = orient * Vec3::NEG_Z;
    let up = orient * Vec3::Y;
    let view = Mat4::look_to_rh(pos, forward, up);
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
        let m = perspective_view_proj(
            Vec3::new(0.0, 5.0, 15.0),
            Quat::IDENTITY,
            60_f32.to_radians(),
            16.0 / 9.0,
        );
        for col in m.to_cols_array() {
            assert!(col.is_finite());
        }
    }
}
