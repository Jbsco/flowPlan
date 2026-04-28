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

/// Reference speed (m/s) for parametric scaling. Rotation rate ω,
/// thickness, and alpha all calibrated such that this speed produces
/// the canonical visible vortex character. Per playtest 2026-04-27:
/// 35 m/s (~70 kts) — typical bushplane mid-cruise / approach speed.
/// At this speed `speed_factor = 1.0` and the ribbon hits its target
/// thickness + rotation rate.
const REFERENCE_CRUISE_MPS: f32 = 35.0;

/// Speed (m/s) at which rotation rate ω → 0. Modeled after the
/// real-world physics intuition that very high airspeed shears the
/// vortex flat (low-AoA cruise = gentle rotation per unit air; near
/// Vne the visible spiral character disappears entirely). Roughly
/// the Vne / max-cruise speed for a Beaver-class GA aircraft.
const ROTATION_ZERO_SPEED_MPS: f32 = 80.0;

/// Mist density threshold for visible vortex. Below = no spawn.
const MIN_MIST_DENSITY: f32 = 0.05;

/// Vertical offset (m, **in body up-direction**) applied to the wingtip
/// spawn position. Applied via `vehicle_orient × Vec3::Y` so the offset
/// rotates with the aircraft — at landing approach (high-AoA), the
/// offset stays perpendicular to the wing rather than world-vertical.
///
/// **Tuning history**:
/// - 0.05 m (ship-1, world-Y): origin appeared low on visible mesh
/// - 0.20 m (ship-2, world-Y): better but orientation confusion at `AoA`
/// - 0.20 m (ship-3 2026-04-27, **body-Y**): now follows wing during
///   pitch/bank — vortex stays attached to wingtip in flight frame
const WINGTIP_VERTICAL_OFFSET_M: f32 = 0.20;

/// Forward/aft offset (m, in body forward direction) for the wingtip
/// spawn. Negative = aft of vehicle origin (typical for wing
/// trailing-edge alignment on a Beaver-like aircraft). Currently 0
/// — pending playtest tune to align with visible wingtip mesh.
const WINGTIP_FORWARD_OFFSET_M: f32 = 0.0;

/// Ribbon lifetime (s). After this the waypoint is dropped.
/// Combined with rotation rate, sets the number of visible loops:
/// `total_rotation = ω × lifetime`. With ω ≈ 29 rad/s at landing
/// (28 m/s) and lifetime 0.8 s, that's ~3.7 complete loops — the
/// playtest target ("3-4 visible loops at slow flight").
///
/// **Tuning history**:
/// - 3.0 s initial: visible trail too long; loops too sparse
/// - 1.5 s (ship-2 2026-04-27): "shorter/more compressed" — landed
///   at ~6 visible loops at landing speed, still too dense
/// - 0.8 s (ship-3 2026-04-27): playtest target — 3-4 loops at slow
///   flight, ~3 at reference cruise. Combined with the sinusoidal
///   size envelope (peak at `life_t` = 0.5), the ribbon now reads
///   as a swell-and-fade rather than a uniformly-thick spiral.
const RIBBON_LIFETIME_S: f32 = 0.8;

/// Rotation rate ω (rad/s) at `REFERENCE_CRUISE_MPS`.
///
/// Higher ω = more loops per second of flight = tighter / sharper
/// spiral. The actual ω used per-frame is interpolated based on
/// airspeed (see `generate_ribbon`):
/// - 25 rad/s at 35 m/s reference (target — visible dramatic helix)
/// - 0 rad/s at `ROTATION_ZERO_SPEED_MPS` (high-speed cruise; the
///   spiral character disappears as airflow shears the vortex flat)
/// - up to ~2× this at very low speed (slow-flight high-AoA = strong
///   rotation per unit air)
///
/// **Tuning history**:
/// - 5 rad/s initial constant: too sparse, only ~2 loops visible
/// - 25 rad/s constant (ship-1): "sharper angle" achieved; matched user spec
/// - 25 rad/s at reference, **inverse-speed scaled** (ship-2 2026-04-27): real-world
///   physics intuition — rotation strength tracks `AoA` / lift coefficient,
///   which is high at slow flight and low at cruise.
const ROTATION_RATE_AT_REFERENCE: f32 = 25.0;

/// Inner radius (m) at envelope baseline. Ribbon's inner edge starts
/// already off-axis so the spiral is visible immediately at the
/// wingtip. Northlander-relative; multiplied by `wing_size_factor`
/// and by the per-waypoint `size_amplitude` envelope (1.0 at age = 0
/// and age = lifetime, peaking mid-life with `peak_factor` swell).
const INNER_RADIUS_BASE_M: f32 = 0.12;

/// Outer-minus-inner thickness (m) at envelope baseline at reference
/// cruise. Modulated inversely by speed via `thickness_speed_factor`
/// (slow flight = thicker; the central 60-70% of the wing section
/// produces a strong force differential at high `AoA`, expanding the
/// visible vapor) AND by the size envelope (sinusoidal swell over
/// the waypoint's lifetime).
const RIBBON_THICKNESS_BASE_M: f32 = 0.35;

/// Peak amplification of the sinusoidal size envelope at slow flight.
/// At reference cruise + above, `peak_factor` = 1.0 (no envelope swell;
/// ribbon stays at baseline thickness over its lifetime). Below
/// reference, `peak_factor` ramps to this value at + below
/// `REFERENCE_CRUISE_MPS - PEAK_RAMP_WIDTH_MPS`. The envelope shape
/// is `sin(π × life_t)`: zero at age 0 and age = lifetime, peaking at
/// `life_t = 0.5`. So at slow flight, the ribbon visibly swells from
/// baseline to ~10× near the middle of each waypoint's life and fades
/// back; combined with the alpha fade, this reads as a dramatic
/// "vortex blooms in the wake" character at landing speed.
const PEAK_AMPLIFICATION_AT_SLOW: f32 = 10.0;

/// Speed deficit (m/s) below `REFERENCE_CRUISE_MPS` at which
/// `peak_factor` saturates at `PEAK_AMPLIFICATION_AT_SLOW`. Linear
/// ramp from 1.0 at reference to 10.0 at reference - this width.
/// Calibrated so that ~28 m/s landing approach (7 m/s below the
/// 35 m/s reference) hits the full 10× swell.
const PEAK_RAMP_WIDTH_MPS: f32 = 7.0;

/// Number of waypoints per second per wingtip. Higher ω needs more
/// sample density to read smoothly (avoid aliasing the rotation).
/// At 60 fps display, 120 wp/s = 2 waypoints per frame. Total
/// waypoints in buffer: lifetime × rate = 1.5 × 120 = 180 per wing.
const WAYPOINTS_PER_SEC: f32 = 120.0;

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

        // Append new waypoints when gates pass.
        if gates_pass {
            // Body-frame axes in world space. `right_axis` is body X
            // (lateral); `up_axis` is body Y; `forward_axis` is body
            // forward (-Z in body frame per the engine's
            // perspective_view_proj convention). Vertical offset is
            // applied along body-Y (rotates with the aircraft) so at
            // high-AoA / banked attitudes the offset stays
            // perpendicular to the wing rather than fixed world-up.
            let right_axis = vehicle_orient * Vec3::X;
            let up_axis = vehicle_orient * Vec3::Y;
            let forward_axis = vehicle_orient * Vec3::NEG_Z;
            let body_offset =
                up_axis * WINGTIP_VERTICAL_OFFSET_M + forward_axis * WINGTIP_FORWARD_OFFSET_M;
            let left_wingtip = vehicle_pos - right_axis * half_span_m + body_offset;
            let right_wingtip = vehicle_pos + right_axis * half_span_m + body_offset;
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
            speed,
            wing_size_factor,
            &mut verts,
        );
        #[allow(clippy::cast_possible_truncation)]
        let left_count = verts.len() as u32;
        Self::generate_ribbon(
            &self.right.waypoints,
            current_time_s,
            speed,
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
    ///
    /// Speed coupling per playtest 2026-04-27 (post-tune):
    /// - **Rotation rate ω scales inversely with speed** — `ω = 25 ×
    ///   max(0, (80 - speed) / (80 - 35))`, capped. 25 rad/s at
    ///   35 m/s; 0 at 80 m/s. Slow flight = tight visible spiral
    ///   (high-`AoA` wing produces strong rotation per unit air);
    ///   high-speed cruise = the spiral character disappears.
    /// - **Ribbon thickness inversely with speed**: slow flight
    ///   produces wider ribbons (the central 60-70% of the wing
    ///   section's force differential has more time per unit travel
    ///   to expand the vapor).
    /// - **Sinusoidal size envelope at slow flight**: at ≤ ~28 m/s
    ///   the ribbon swells from baseline to 10× peak at `life_t = 0.5`
    ///   then fades back — the dramatic "vortex blooms in the wake"
    ///   character of low-speed bushplane flight in mist.
    /// - **Alpha inversely with speed**: high speed → transparent
    ///   subtle hint; slow flight → opaque dramatic.
    fn generate_ribbon(
        waypoints: &VecDeque<Waypoint>,
        current_time_s: f32,
        speed_mps: f32,
        wing_size_factor: f32,
        out: &mut Vec<RibbonVertexGpu>,
    ) {
        if waypoints.len() < 2 {
            return;
        }
        let world_up = Vec3::Y;
        // Speed-driven rotation rate: linear ramp from 0 at
        // `ROTATION_ZERO_SPEED_MPS` to `ROTATION_RATE_AT_REFERENCE`
        // at `REFERENCE_CRUISE_MPS`, allowed to extrapolate higher
        // for slow flight (capped at 2× to avoid blur).
        let speed_lerp = ((ROTATION_ZERO_SPEED_MPS - speed_mps)
            / (ROTATION_ZERO_SPEED_MPS - REFERENCE_CRUISE_MPS))
            .clamp(0.0, 2.0);
        let omega = ROTATION_RATE_AT_REFERENCE * speed_lerp;
        // Speed-driven thickness multiplier — asymmetric around
        // reference. Below reference (slow flight) the curve is
        // CUBIC-inverse so thickness grows aggressively at low
        // speeds — the central 60-70% of the wing section produces
        // a strong force differential at high AoA, expanding the
        // visible vapor much faster than at cruise. Above reference
        // (fast cruise) the curve is linear-inverse capped at the
        // reference-baseline (no thicker than at reference) and
        // floored at 0.5 (no thinner than half).
        let thickness_speed_factor = if speed_mps >= REFERENCE_CRUISE_MPS {
            (REFERENCE_CRUISE_MPS / speed_mps).clamp(0.5, 1.0)
        } else {
            (REFERENCE_CRUISE_MPS / speed_mps.max(MIN_PRODUCTION_SPEED_MPS))
                .powi(3)
                .min(8.0)
        };
        // Speed-driven peak amplification — applied to a sinusoidal
        // envelope over the waypoint's lifetime. Slow flight produces
        // a dramatic swell (peak ~10× baseline at life_t = 0.5) that
        // builds and fades; reference-and-above stay flat (peak = 1).
        // Linear ramp from 1.0 at `REFERENCE_CRUISE_MPS` to
        // `PEAK_AMPLIFICATION_AT_SLOW` at
        // `REFERENCE_CRUISE_MPS - PEAK_RAMP_WIDTH_MPS`, saturating at
        // `PEAK_AMPLIFICATION_AT_SLOW` for any further speed deficit.
        let peak_factor = if speed_mps >= REFERENCE_CRUISE_MPS {
            1.0
        } else {
            let deficit =
                ((REFERENCE_CRUISE_MPS - speed_mps) / PEAK_RAMP_WIDTH_MPS).clamp(0.0, 1.0);
            1.0 + (PEAK_AMPLIFICATION_AT_SLOW - 1.0) * deficit
        };
        // Speed-driven alpha attenuation — opaque at slow speed,
        // transparent at fast cruise. Same shape as for thickness/
        // peak_factor: quietly subtle at high speed, dramatic at slow.
        let speed_factor = (speed_mps / REFERENCE_CRUISE_MPS).clamp(0.5, 2.5);
        let alpha_speed_factor = (1.4 - speed_factor * 0.4).clamp(0.4, 1.4);
        for wp in waypoints {
            let age = current_time_s - wp.spawn_time_s;
            if age < 0.0 {
                continue;
            }
            // Parametric spiral. Rotation θ = ω(speed) × age.
            let theta = omega * age;
            // Per-waypoint life parameter (0 = just spawned;
            // 1 = end-of-life). Drives the sinusoidal size envelope
            // and the alpha fade.
            let life_t = (age / RIBBON_LIFETIME_S).clamp(0.0, 1.0);
            // Sinusoidal envelope: 0 at the endpoints (life_t = 0,
            // life_t = 1), peaking at 1 at life_t = 0.5.
            let envelope = (life_t * core::f32::consts::PI).sin();
            // Combined size scaler. At reference+: peak_factor = 1
            // → size_amplitude flat 1.0 (no envelope swell). At
            // slow flight: peak_factor = 10 → size_amplitude swells
            // from 1 → 10 → 1 over the waypoint's life.
            let size_amplitude = 1.0 + (peak_factor - 1.0) * envelope;
            let r_inner = INNER_RADIUS_BASE_M * wing_size_factor * size_amplitude;
            let thickness = RIBBON_THICKNESS_BASE_M * thickness_speed_factor * size_amplitude;
            let r_outer = r_inner + thickness * wing_size_factor;

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
            // for soft tail. Modulated by speed (high speed = more
            // transparent overall; subtle hint at cruise vs opaque
            // dramatic at slow flight).
            let age_alpha = 1.0 - smoothstep(0.4, 1.0, life_t);
            let alpha = (age_alpha * alpha_speed_factor).clamp(0.0, 1.0);

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
