//! Phase 1 renderer: flat-shaded pipeline with depth buffer, camera + model uniforms,
//! terrain + placeholder cube.
//!
//! Construction wires the wgpu pipeline, depth texture, vertex/index buffers for the
//! two meshes, and their uniform bind groups. `resize` recreates the depth buffer.
//! `render` interpolates between two `SimSnapshot`s and submits a single draw pass.

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
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ModelUniform {
    model: [[f32; 4]; 4],
    normal_mat: [[f32; 4]; 4],
    tint: [f32; 4],
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
}

/// Top-level renderer.
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,
    camera_buf: wgpu::Buffer,
    camera_bg: wgpu::BindGroup,
    model_bgl: wgpu::BindGroupLayout,
    terrain: DrawModel,
    vehicle: DrawModel,
    cube_size: f32,
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
        vehicle_mesh: &CpuMesh,
        vehicle_cube_size: f32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-phase1-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Bind group layouts.
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("phase1-pipeline-layout"),
            bind_group_layouts: &[&camera_bgl, &model_bgl],
            push_constant_ranges: &[],
        });

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

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("phase1-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &vertex_buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
                cull_mode: Some(wgpu::Face::Back),
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
        });

        let depth_view = create_depth_view(device, initial_size);

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

        // Terrain draw model (identity transform, light tint).
        let terrain = DrawModel::new(
            device,
            &model_bgl,
            terrain_mesh,
            Mat4::IDENTITY,
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        );

        // Vehicle draw model. Initial transform is identity; `render` overwrites each frame.
        let vehicle = DrawModel::new(
            device,
            &model_bgl,
            vehicle_mesh,
            Mat4::IDENTITY,
            Vec4::new(1.0, 1.0, 1.0, 1.0),
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
            }),
        );

        Self {
            pipeline,
            depth_view,
            camera_buf,
            camera_bg,
            model_bgl,
            terrain,
            vehicle,
            cube_size: vehicle_cube_size,
        }
    }

    /// Handle a window resize.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        self.depth_view = create_depth_view(device, size);
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
        let model_mat =
            Mat4::from_scale_rotation_translation(Vec3::splat(self.cube_size), veh_orient, veh_pos);
        let normal_mat = Mat4::from_quat(veh_orient);
        let vehicle_uniform = ModelUniform {
            model: model_mat.to_cols_array_2d(),
            normal_mat: normal_mat.to_cols_array_2d(),
            tint: [1.0, 0.82, 0.28, 1.0],
        };
        queue.write_buffer(
            &self.vehicle.uniform_buf,
            0,
            bytemuck::bytes_of(&vehicle_uniform),
        );

        // Begin render pass.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("phase1-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
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

        // Draw terrain.
        pass.set_bind_group(1, &self.terrain.bind_group, &[]);
        pass.set_vertex_buffer(0, self.terrain.mesh.vbo.slice(..));
        pass.set_index_buffer(self.terrain.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.terrain.mesh.index_count, 0, 0..1);

        // Draw vehicle cube.
        pass.set_bind_group(1, &self.vehicle.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vehicle.mesh.vbo.slice(..));
        pass.set_index_buffer(self.vehicle.mesh.ibo.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.vehicle.mesh.index_count, 0, 0..1);
    }
}

impl DrawModel {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        mesh: &CpuMesh,
        initial_model: Mat4,
        tint: Vec4,
    ) -> Self {
        let mesh_gpu = GpuMesh::upload(device, mesh);
        let uniform = ModelUniform {
            model: initial_model.to_cols_array_2d(),
            normal_mat: initial_model.to_cols_array_2d(),
            tint: [tint.x, tint.y, tint.z, tint.w],
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
        }
    }
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    tex.create_view(&wgpu::TextureViewDescriptor::default())
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
