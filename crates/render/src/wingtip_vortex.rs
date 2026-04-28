//! Wingtip vortex ribbon visualization (Phase 3.2x.W, 2026-04-27).
//!
//! Renders a continuous parametric ribbon helix per wingtip — a single
//! surface wrapped around the aircraft's flight path, twisting around
//! the spine via the parametric pair:
//!
//! ```text
//!   inner(age) = spine + r_inner(age) · (cos θ(age) · perp_a + sin θ(age) · perp_b)
//!   outer(age) = spine + r_outer(age) · (cos θ(age) · perp_a + sin θ(age) · perp_b)
//! ```
//!
//! Two circular functions (inner + outer) at the same angle but
//! different radii. The line between inner and outer at each waypoint
//! is one ribbon "rung"; consecutive rungs form a triangle strip —
//! a smooth 3D ribbon spiraling around the spine.
//!
//! # Why this replaces the particle pool (v1)
//!
//! The particle approach (8 puffs per ring × N rings/sec) produces
//! visibly DISCRETE rings rather than a continuous tube. Even at high
//! density, the gap between rings reads as "string of bubble rings"
//! rather than "wingtip vortex." The ribbon gives a single connected
//! surface — exactly the spiral helix shape from the reference image
//! (vortex visible in misty air).
//!
//! # Spawn discipline (preserved from particle v1)
//!
//! Waypoints are appended only when:
//! - airspeed ≥ `MIN_PRODUCTION_SPEED_MPS` (no vortex below stall-ish)
//! - mist density at vehicle position ≥ `MIN_MIST_DENSITY` (clear-air
//!   vortices invisible — matches real-world condensation physics)
//!
//! When either gate fails, no new waypoints are added. Existing
//! waypoints continue their lifetime fade (so the ribbon trails off
//! naturally as the aircraft slows or leaves mist).
//!
//! # Aircraft-spec hooks
//!
//! Particle radius scales with `wing_size_factor` derived from
//! `(wing_area × aspect_ratio).sqrt() / 2` (half-span). Future v2
//! could add airfoil-driven `CL_max` from `airfoil_root` /
//! `airfoil_tip` NACA designations to spike vortex intensity at
//! high-AoA / stalled flight.

use std::collections::VecDeque;

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use wgpu::util::DeviceExt;

/// Vertex buffer capacity (vertices, not waypoints). Each waypoint
/// emits 2 vertices (inner + outer ribbon edge). Sized for the
/// max-lifetime × max-fps × 2 wings × 2 verts/waypoint case.
///
/// At 60 fps × 3 s lifetime × 2 wings × 2 verts = 720. 2048 gives
/// 2.8× burst margin (e.g. 144 fps test scene).
pub const WINGTIP_VERTEX_CAPACITY: usize = 2048;

/// Pool size — backwards-compat alias used by `lib.rs` exports.
/// Same numeric value as the vertex capacity.
pub const WINGTIP_POOL_SIZE: usize = WINGTIP_VERTEX_CAPACITY;

/// Minimum airspeed (m/s) to produce visible vortices. Below this no
/// circulation is strong enough to register even in mist.
const MIN_PRODUCTION_SPEED_MPS: f32 = 15.0;

/// Reference cruise speed for parametric scaling. Rotation rate ω,
/// initial radius, expansion all scale relative to this.
const REFERENCE_CRUISE_MPS: f32 = 45.0;

/// Mist density threshold for visible vortex. Below = no spawn.
const MIN_MIST_DENSITY: f32 = 0.05;

/// Vertical offset (m) applied to the wingtip spawn position to match
/// the visible mesh. See note in v1; calibrated for Northlander.
const WINGTIP_VERTICAL_OFFSET_M: f32 = 0.05;

/// Particle/ribbon lifetime (s). After this the waypoint is dropped.
/// Combined with rotation rate, sets the number of visible loops:
/// `total_rotation = ω × lifetime`. With ω ≈ 5 rad/s and lifetime
/// 3 s, that's ~2.4 complete loops — matches "fade out after a few
/// complete loops" from design discussion.
const RIBBON_LIFETIME_S: f32 = 3.0;

/// Rotation rate ω at reference cruise (rad/s). Faster aircraft →
/// tighter spiral. The angle along the ribbon is θ(age) = ω · age,
/// scaled by speed.
const ROTATION_RATE_AT_REFERENCE: f32 = 5.0;

/// Initial inner radius (m) at age 0. Ribbon's inner edge starts
/// tight against the spine. Northlander-relative; multiplied by
/// `wing_size_factor` for larger aircraft.
const INNER_RADIUS_BASE_M: f32 = 0.10;

/// Initial outer-vs-inner radius offset (m). Defines starting ribbon
/// thickness. Inner = `INNER_RADIUS_BASE_M`; outer = inner + this.
const RIBBON_THICKNESS_BASE_M: f32 = 0.15;

/// Inner radius growth rate (m/s). Vortex spreads outward as it ages.
const INNER_RADIUS_GROWTH_M_PER_S: f32 = 0.05;

/// Outer radius growth rate (m/s). Faster than inner → ribbon WIDENS
/// over age (vortex breakdown — vapor disperses outward).
const OUTER_RADIUS_GROWTH_M_PER_S: f32 = 0.15;

/// Number of waypoints per second per wingtip. Higher = smoother
/// ribbon (shorter triangle strip segments). At 60 fps, 60 wp/s
/// gives one waypoint per frame. Lifetime × rate = total waypoints
/// in the buffer (3 s × 60 = 180 per wing).
const WAYPOINTS_PER_SEC: f32 = 60.0;

/// Per-vertex GPU layout: world position + alpha = 4 floats = 16 B.
/// Padded to 16 B (already aligned).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Default)]
struct RibbonVertexGpu {
    world_pos: [f32; 3],
    alpha: f32,
}

#[derive(Clone, Copy, Debug)]
struct Waypoint {
    spine_pos: Vec3,
    spawn_time_s: f32,
    /// Aircraft direction-of-travel axis at spawn. Used to compute
    /// the perpendicular plane in which the spiral wraps.
    axis: Vec3,
}

/// Per-wing ring buffer of waypoints. New waypoints append to the
/// back; expired waypoints are drained from the front.
struct WingState {
    waypoints: VecDeque<Waypoint>,
    /// Carry-over for sub-frame waypoint spacing.
    spawn_accumulator: f32,
}

impl WingState {
    fn new() -> Self {
        Self {
            waypoints: VecDeque::with_capacity(256),
            spawn_accumulator: 0.0,
        }
    }
}

/// Wingtip vortex ribbon system. Maintains separate waypoint buffers
/// for left + right wing; regenerates the GPU vertex buffer per frame.
pub struct WingtipVortexSystem {
    left: WingState,
    right: WingState,
    vertex_buffer: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    /// Vertex count for left ribbon (drawn first).
    left_vertex_count: u32,
    /// Vertex count for right ribbon (drawn second; offset = `left_vertex_count`).
    right_vertex_count: u32,
    last_update_time_s: f32,
}

impl WingtipVortexSystem {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wingtip-vortex-vb"),
            // Allocate full capacity now; per-frame writes reuse the buffer.
            contents: &vec![
                0_u8;
                WINGTIP_VERTEX_CAPACITY * core::mem::size_of::<RibbonVertexGpu>()
            ],
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wingtip-vortex-shader"),
            source: wgpu::ShaderSource::Wgsl(crate::WINGTIP_VORTEX_WGSL_SOURCE.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wingtip-vortex-pipeline-layout"),
            bind_group_layouts: &[camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wingtip-vortex-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: core::mem::size_of::<RibbonVertexGpu>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        // world_pos
                        wgpu::VertexAttribute {
                            shader_location: 0,
                            offset: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        // alpha
                        wgpu::VertexAttribute {
                            shader_location: 1,
                            offset: 12,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                }],
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
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // ribbon is double-sided
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
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

        Self {
            left: WingState::new(),
            right: WingState::new(),
            vertex_buffer,
            pipeline,
            left_vertex_count: 0,
            right_vertex_count: 0,
            last_update_time_s: 0.0,
        }
    }

    /// Per-frame update.
    ///
    /// Appends new waypoints to each wing buffer (gated by speed +
    /// mist), drops expired waypoints, regenerates the ribbon vertex
    /// buffer from the active waypoints, uploads to the GPU.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        vehicle_pos: Vec3,
        vehicle_orient: Quat,
        vehicle_velocity: Vec3,
        half_span_m: f32,
        wing_size_factor: f32,
        mist_density: f32,
        current_time_s: f32,
    ) {
        let dt = (current_time_s - self.last_update_time_s).clamp(0.0, 0.1);
        self.last_update_time_s = current_time_s;

        let speed = vehicle_velocity.length();
        let gates_pass = speed >= MIN_PRODUCTION_SPEED_MPS && mist_density >= MIN_MIST_DENSITY;
        let speed_factor = (speed / REFERENCE_CRUISE_MPS).clamp(0.5, 2.5);

        // Append new waypoints when gates pass.
        if gates_pass {
            let right_axis = vehicle_orient * Vec3::X;
            let vertical_offset = Vec3::new(0.0, WINGTIP_VERTICAL_OFFSET_M, 0.0);
            let left_wingtip = vehicle_pos - right_axis * half_span_m + vertical_offset;
            let right_wingtip = vehicle_pos + right_axis * half_span_m + vertical_offset;
            let velocity_axis = if speed > 0.1 {
                vehicle_velocity / speed
            } else {
                vehicle_orient * Vec3::NEG_Z
            };

            self.left.spawn_accumulator += WAYPOINTS_PER_SEC * dt;
            self.right.spawn_accumulator += WAYPOINTS_PER_SEC * dt;
            #[allow(clippy::cast_possible_truncation)]
            #[allow(clippy::cast_sign_loss)]
            let left_count = self.left.spawn_accumulator.floor() as u32;
            self.left.spawn_accumulator -= left_count as f32;
            for _ in 0..left_count {
                self.left.waypoints.push_back(Waypoint {
                    spine_pos: left_wingtip,
                    spawn_time_s: current_time_s,
                    axis: velocity_axis,
                });
            }
            #[allow(clippy::cast_possible_truncation)]
            #[allow(clippy::cast_sign_loss)]
            let right_count = self.right.spawn_accumulator.floor() as u32;
            self.right.spawn_accumulator -= right_count as f32;
            for _ in 0..right_count {
                self.right.waypoints.push_back(Waypoint {
                    spine_pos: right_wingtip,
                    spawn_time_s: current_time_s,
                    axis: velocity_axis,
                });
            }
        } else {
            self.left.spawn_accumulator = 0.0;
            self.right.spawn_accumulator = 0.0;
        }

        // Drop expired waypoints (oldest-first).
        let cutoff_time = current_time_s - RIBBON_LIFETIME_S;
        while let Some(front) = self.left.waypoints.front() {
            if front.spawn_time_s < cutoff_time {
                self.left.waypoints.pop_front();
            } else {
                break;
            }
        }
        while let Some(front) = self.right.waypoints.front() {
            if front.spawn_time_s < cutoff_time {
                self.right.waypoints.pop_front();
            } else {
                break;
            }
        }

        // Generate ribbon vertices: 2 verts per waypoint (inner + outer).
        let mut verts: Vec<RibbonVertexGpu> = Vec::with_capacity(WINGTIP_VERTEX_CAPACITY);
        Self::generate_ribbon(
            &self.left.waypoints,
            current_time_s,
            speed_factor,
            wing_size_factor,
            &mut verts,
        );
        #[allow(clippy::cast_possible_truncation)]
        let left_count = verts.len() as u32;
        Self::generate_ribbon(
            &self.right.waypoints,
            current_time_s,
            speed_factor,
            wing_size_factor,
            &mut verts,
        );
        #[allow(clippy::cast_possible_truncation)]
        let total_count = verts.len() as u32;
        let right_count = total_count - left_count;

        self.left_vertex_count = left_count;
        self.right_vertex_count = right_count;

        if total_count > 0 {
            // Cap to capacity defensively (shouldn't fire in practice).
            let upload_count = (total_count as usize).min(WINGTIP_VERTEX_CAPACITY);
            queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&verts[..upload_count]),
            );
        }
    }

    /// Generate triangle-strip vertices for one wing's ribbon.
    /// Appends to `out`; needs at least 2 waypoints to produce visible
    /// triangles (a strip of 1 quad = 4 verts).
    fn generate_ribbon(
        waypoints: &VecDeque<Waypoint>,
        current_time_s: f32,
        speed_factor: f32,
        wing_size_factor: f32,
        out: &mut Vec<RibbonVertexGpu>,
    ) {
        if waypoints.len() < 2 {
            return;
        }
        let world_up = Vec3::Y;
        for wp in waypoints {
            let age = current_time_s - wp.spawn_time_s;
            if age < 0.0 {
                continue;
            }
            // Parametric spiral: angle and radii functions of age.
            let theta = ROTATION_RATE_AT_REFERENCE * speed_factor * age;
            let r_inner =
                (INNER_RADIUS_BASE_M + INNER_RADIUS_GROWTH_M_PER_S * age) * wing_size_factor;
            let r_outer =
                (INNER_RADIUS_BASE_M + RIBBON_THICKNESS_BASE_M + OUTER_RADIUS_GROWTH_M_PER_S * age)
                    * wing_size_factor;

            // Perpendicular plane to wp.axis.
            let mut perp_a = wp.axis.cross(world_up);
            if perp_a.length_squared() < 0.001 {
                perp_a = Vec3::X;
            } else {
                perp_a = perp_a.normalize();
            }
            let perp_b = wp.axis.cross(perp_a).normalize();
            let (sin_t, cos_t) = theta.sin_cos();
            let radial_dir = perp_a * cos_t + perp_b * sin_t;
            let inner_pos = wp.spine_pos + radial_dir * r_inner;
            let outer_pos = wp.spine_pos + radial_dir * r_outer;

            // Alpha fade — full at age 0, zero at lifetime, smoothstep
            // for soft tail.
            let life_t = (age / RIBBON_LIFETIME_S).clamp(0.0, 1.0);
            let alpha = 1.0 - smoothstep(0.4, 1.0, life_t);

            out.push(RibbonVertexGpu {
                world_pos: [inner_pos.x, inner_pos.y, inner_pos.z],
                alpha,
            });
            out.push(RibbonVertexGpu {
                world_pos: [outer_pos.x, outer_pos.y, outer_pos.z],
                alpha,
            });
        }
    }

    pub fn record<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera_bg: &'a wgpu::BindGroup) {
        let total = self.left_vertex_count + self.right_vertex_count;
        if total == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bg, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        // Two separate triangle-strip draws so the right ribbon doesn't
        // connect to the left ribbon's last vertex.
        if self.left_vertex_count >= 4 {
            pass.draw(0..self.left_vertex_count, 0..1);
        }
        if self.right_vertex_count >= 4 {
            pass.draw(self.left_vertex_count..total, 0..1);
        }
    }

    /// Live waypoint count across both wings — debug HUD readout.
    /// Each waypoint emits 2 vertices, so the GPU vertex count is 2×
    /// this. Reported in waypoints (not vertices) for clarity.
    #[must_use]
    pub fn active_count(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation)]
        let count = self.left.waypoints.len() + self.right.waypoints.len();
        count as u32
    }
}

#[allow(clippy::manual_clamp)]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
