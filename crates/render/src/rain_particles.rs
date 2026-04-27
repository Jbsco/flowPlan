//! World-space rain particle pool (Post-CM-10 Phase E.4, 2026-04-27).
//!
//! Renders deterministic-but-not-replay-critical raindrop billboards
//! anchored to world coordinates around the camera. Drops fall at
//! terminal velocity and recycle through a fixed pool when they pass
//! below the camera or exceed lifetime. Density scales with the
//! `precip_intensity` plumbed through `SceneParams`.
//!
//! # Why a real particle pool
//!
//! Earlier shader-only attempts (screen-space hash overlay + multi-
//! distance world-space sampling in `volumetric.wgsl`) couldn't give
//! correct motion parallax: drops appeared to scroll with camera
//! position rather than streak past the viewer. A particle pool fixes
//! this: each drop has a fixed world position, so projection through
//! the view matrix produces real perspective-driven parallax.
//!
//! # Determinism
//!
//! Rain is purely visual — it doesn't feed sim or audio. Spawn
//! positions use a per-frame xorshift; pool order is preserved across
//! frames, so individual drops have stable trajectories until they
//! recycle. Replay safety is deferred (Phase 6) since spawn jitter is
//! invisible past one frame.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

/// Maximum particle pool size.
///
/// 4096 is enough drops for "in heavy rain" visual density at typical
/// view distances (10-50 m streaks visible). Buffer is
/// `4096 × 16 B = 64 KiB` — trivial. Active count scales with
/// `precip_intensity / 14 mm/hr × MAX`.
pub const RAIN_PARTICLE_POOL_SIZE: usize = 4096;

/// Radius (m) of the cylindrical spawn region around the camera. Drops
/// outside this radius are recycled to a new spawn position. 30 m is
/// tight enough to keep drops visually dense around the camera (most
/// drops within 30 m are within typical raymarched-cloud detail range);
/// large enough that an aircraft at 30 m/s still sees consistent rain
/// for ~1 second ahead before drops respawn.
const SPAWN_RADIUS_M: f32 = 30.0;

/// Spawn altitude (m) above the camera. Drops fall from here at
/// terminal velocity. Combined with `RECYCLE_BELOW_CAM_M`, gives a
/// ~40 m falling distance — drops exit the view region rather than
/// piling up on the ground.
const SPAWN_HEIGHT_ABOVE_CAM_M: f32 = 30.0;

/// Recycle a drop when its current Y is this far below the camera.
const RECYCLE_BELOW_CAM_M: f32 = 10.0;

/// Recycle a drop after this many seconds regardless of position
/// (insurance against drops lingering far from camera due to camera
/// movement).
const MAX_PARTICLE_LIFETIME_S: f32 = 5.0;

/// Visual fall speed for rain particles at `snow_fraction = 0` (m/s).
///
/// Real raindrop terminal velocity is 4-9 m/s, but rendered drops at
/// 5-30 m camera distance LOOK slow at that speed because we lack the
/// near-eye motion blur of real rain. Bumped to 18 m/s for visual
/// punch — drops streak past the camera quickly enough to read as
/// "moving rain" rather than "drifting particles."
const RAIN_FALL_SPEED_M_PER_S: f32 = 18.0;

/// Visual fall speed for rain particles at `snow_fraction = 1` (m/s).
///
/// Snowflakes drift down at ~0.5-1.5 m/s in real life. Pick the upper
/// end so drift is visible without taking forever to fall.
const SNOW_FALL_SPEED_M_PER_S: f32 = 1.5;

/// Per-instance GPU layout. Just the world position — quad geometry
/// is built per-vertex in the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Default)]
struct RainInstanceGpu {
    world_pos: [f32; 3],
    _padding: f32,
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    seed_pos: Vec3,
    seed_time_s: f32,
}

/// World-space rain particle system. Owns the GPU buffers + pipeline.
pub struct RainParticleSystem {
    particles: Vec<Particle>,
    instance_buffer: wgpu::Buffer,
    quad_vertex_buffer: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    active_count: u32,
    rng_state: u64,
    last_camera_pos: Vec3,
}

impl RainParticleSystem {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        scene_bind_group_layout: &wgpu::BindGroupLayout,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        // Quad: two triangles forming a [-1, +1] × [-1, +1] square.
        // Each vertex carries its (x, y) corner offset; the shader
        // multiplies these by the streak half-extents to build the
        // billboard.
        #[rustfmt::skip]
        let quad_verts: [[f32; 2]; 6] = [
            [-1.0, -1.0],
            [ 1.0, -1.0],
            [ 1.0,  1.0],
            [-1.0, -1.0],
            [ 1.0,  1.0],
            [-1.0,  1.0],
        ];
        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rain-quad-vb"),
            contents: bytemuck::cast_slice(&quad_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rain-instance-buf"),
            size: (RAIN_PARTICLE_POOL_SIZE * core::mem::size_of::<RainInstanceGpu>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rain-particles-shader"),
            source: wgpu::ShaderSource::Wgsl(crate::RAIN_PARTICLES_WGSL_SOURCE.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rain-particles-pipeline-layout"),
            bind_group_layouts: &[camera_bind_group_layout, scene_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rain-particles-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    // Per-vertex quad corner.
                    wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            shader_location: 0,
                            offset: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        }],
                    },
                    // Per-instance world position.
                    wgpu::VertexBufferLayout {
                        array_stride: core::mem::size_of::<RainInstanceGpu>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            shader_location: 1,
                            offset: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        }],
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let mut particles = Vec::with_capacity(RAIN_PARTICLE_POOL_SIZE);
        for _ in 0..RAIN_PARTICLE_POOL_SIZE {
            // Initial state: dead (seed_time in the past, will recycle
            // on first update).
            particles.push(Particle {
                seed_pos: Vec3::ZERO,
                seed_time_s: -1000.0,
            });
        }

        Self {
            particles,
            instance_buffer,
            quad_vertex_buffer,
            pipeline,
            active_count: 0,
            rng_state: 0x9E37_79B9_7F4A_7C15,
            last_camera_pos: Vec3::ZERO,
        }
    }

    fn next_rand_unit(&mut self) -> f32 {
        // xorshift64 → [0, 1) f32.
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        // High 24 bits → uniform [0, 1)
        ((x >> 40) as u32) as f32 / 16_777_216.0
    }

    fn fresh_spawn_pos(&mut self, camera_pos: Vec3) -> Vec3 {
        // Uniform disc sample around camera xz, with random altitude
        // jitter so drops don't all start at the same Y.
        let theta = self.next_rand_unit() * core::f32::consts::TAU;
        let r = self.next_rand_unit().sqrt() * SPAWN_RADIUS_M;
        let (sin_t, cos_t) = theta.sin_cos();
        let y_jitter = self.next_rand_unit() * 10.0;
        Vec3::new(
            camera_pos.x + r * cos_t,
            camera_pos.y + SPAWN_HEIGHT_ABOVE_CAM_M + y_jitter,
            camera_pos.z + r * sin_t,
        )
    }

    /// Per-frame update. Advances each active particle, recycles ones
    /// past their lifetime / below the camera, and uploads positions to
    /// the GPU.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera_pos: Vec3,
        precip_intensity: f32,
        snow_fraction: f32,
        current_time_s: f32,
    ) {
        // Active fraction of the pool — full at 14 mm/hr (squall), half
        // at 7 mm/hr, ~2% at light drizzle.
        let intensity_fraction = (precip_intensity / 14.0).clamp(0.0, 1.0);
        #[allow(clippy::cast_precision_loss)]
        #[allow(clippy::cast_sign_loss)]
        #[allow(clippy::cast_possible_truncation)]
        let active = (intensity_fraction * RAIN_PARTICLE_POOL_SIZE as f32) as usize;
        self.active_count = active as u32;
        self.last_camera_pos = camera_pos;

        if active == 0 {
            return;
        }

        // Phase E.3: lerp fall speed between rain (fast streaks) and
        // snow (slow drift) by `snow_fraction`. Linear blend is fine
        // here since both endpoints are tuned visually, not from
        // physics.
        let snow = snow_fraction.clamp(0.0, 1.0);
        let fall_speed = RAIN_FALL_SPEED_M_PER_S * (1.0 - snow) + SNOW_FALL_SPEED_M_PER_S * snow;

        let mut instance_data: Vec<RainInstanceGpu> = Vec::with_capacity(active);
        for i in 0..active {
            let (mut current, mut elapsed, seed_pos, seed_time_s) = {
                let p = &self.particles[i];
                let e = current_time_s - p.seed_time_s;
                let c = p.seed_pos + Vec3::new(0.0, -fall_speed * e, 0.0);
                (c, e, p.seed_pos, p.seed_time_s)
            };
            let _ = (seed_pos, seed_time_s);
            let dx = current.x - camera_pos.x;
            let dz = current.z - camera_pos.z;
            let r_sq = dx * dx + dz * dz;
            let recycle = current.y < camera_pos.y - RECYCLE_BELOW_CAM_M
                || elapsed > MAX_PARTICLE_LIFETIME_S
                || r_sq > (SPAWN_RADIUS_M * 1.5) * (SPAWN_RADIUS_M * 1.5);
            if recycle {
                let new_seed_pos = self.fresh_spawn_pos(camera_pos);
                let lifetime_offset = self.next_rand_unit() * 2.0;
                let new_seed_time = current_time_s - lifetime_offset;
                self.particles[i].seed_pos = new_seed_pos;
                self.particles[i].seed_time_s = new_seed_time;
                elapsed = current_time_s - new_seed_time;
                current = new_seed_pos + Vec3::new(0.0, -fall_speed * elapsed, 0.0);
            }
            instance_data.push(RainInstanceGpu {
                world_pos: [current.x, current.y, current.z],
                _padding: 0.0,
            });
        }

        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&instance_data),
        );
    }

    /// Record draw call into an active render pass. Caller binds
    /// `camera_bg` at slot 0 + `scene_bg` at slot 1.
    pub fn record<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bg: &'a wgpu::BindGroup,
        scene_bg: &'a wgpu::BindGroup,
    ) {
        if self.active_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bg, &[]);
        pass.set_bind_group(1, scene_bg, &[]);
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..self.active_count);
    }

    pub fn active_count(&self) -> u32 {
        self.active_count
    }
}
