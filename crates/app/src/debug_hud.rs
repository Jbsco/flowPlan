//! Debug HUD: egui overlay showing current sim state.
//!
//! Per [`DESIGN.md § Non-Goals`](../../../../DESIGN.md#non-goals), egui is acceptable for
//! debug overlays only; the real in-game HUD is hand-rolled in Phase 4. This module is
//! the "debug overlay" scope: it reads the latest `SimSnapshot` and prints position,
//! velocity, altitude, attitude, controls, view, and frame time in a top-left panel.
//!
//! No input is consumed -- events still flow to the sim. egui is rendered as a second
//! pass on top of the main scene.

#![deny(unsafe_code)]

use std::time::Instant;

use egui::Context;
use egui_wgpu::{Renderer, ScreenDescriptor};
use egui_winit::State;
use flowplan_render::Telemetry;
use flowplan_sim::{AircraftSpec, StallState};
use winit::event::WindowEvent;
use winit::window::Window;

/// Everything the debug HUD needs per frame, besides the live sim snapshot and window.
pub struct DebugHud {
    ctx: Context,
    state: State,
    renderer: Renderer,
    last_frame_time: Instant,
    /// Rolling average of frame time in milliseconds.
    frame_time_ms_ema: f32,
}

impl DebugHud {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, window: &Window) -> Self {
        let ctx = Context::default();
        let viewport_id = ctx.viewport_id();
        let state = State::new(ctx.clone(), viewport_id, window, None, None, None);
        let renderer = Renderer::new(device, target_format, None, 1, false);
        Self {
            ctx,
            state,
            renderer,
            last_frame_time: Instant::now(),
            frame_time_ms_ema: 16.6,
        }
    }

    /// Forward a winit event to egui. Returns whether egui consumed it (debug HUD never
    /// consumes input in Phase 1, but this is the standard egui integration point).
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Encode the debug HUD into the given command encoder, drawing into `target_view`.
    /// Must be called after the main scene pass so the HUD draws on top.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        window: &Window,
        telemetry: &Telemetry,
        pending_ticks_last_frame: u32,
        aircraft: Option<&AircraftSpec>,
        debug_overlay: Option<&DebugCameraOverlay<'_>>,
    ) {
        let now = Instant::now();
        let dt_ms = now.duration_since(self.last_frame_time).as_secs_f32() * 1000.0;
        self.last_frame_time = now;
        // Exponential moving average (alpha ~ 0.1) for a readable FPS readout.
        self.frame_time_ms_ema = 0.9 * self.frame_time_ms_ema + 0.1 * dt_ms;

        let raw_input = self.state.take_egui_input(window);
        let full_output = self.ctx.run(raw_input, |ctx| {
            if let Some(overlay) = debug_overlay {
                // Debug-camera mode: hide the aircraft flight-instrument
                // panels (dynamics / glide / debug). Show the camera
                // overlay with position / heading / speed-tier / FPS /
                // active-cell-count / rebuild-status.
                build_debug_camera_ui(ctx, self.frame_time_ms_ema, overlay);
            } else {
                build_ui(
                    ctx,
                    telemetry,
                    aircraft,
                    self.frame_time_ms_ema,
                    pending_ticks_last_frame,
                );
            }
        });
        self.state
            .handle_platform_output(window, full_output.platform_output);

        let pixels_per_point = self.state.egui_ctx().pixels_per_point();
        let size = window.inner_size();
        let screen = ScreenDescriptor {
            size_in_pixels: [size.width.max(1), size.height.max(1)],
            pixels_per_point,
        };

        for (tex_id, image_delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(device, queue, *tex_id, image_delta);
        }

        let clipped_primitives = self.ctx.tessellate(full_output.shapes, pixels_per_point);

        self.renderer
            .update_buffers(device, queue, encoder, &clipped_primitives, &screen);

        {
            let mut rpass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("debug-hud-pass"),
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
                })
                .forget_lifetime();
            self.renderer
                .render(&mut rpass, &clipped_primitives, &screen);
        }

        for tex_id in &full_output.textures_delta.free {
            self.renderer.free_texture(tex_id);
        }
    }
}

#[allow(clippy::too_many_lines)]
fn build_ui(
    ctx: &Context,
    t: &Telemetry,
    aircraft: Option<&AircraftSpec>,
    frame_ms: f32,
    pending_ticks: u32,
) {
    egui::Window::new("debug")
        .default_pos([10.0, 10.0])
        .default_width(260.0)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.label(format!("tick: {}", t.tick));
            ui.label(format!(
                "frame: {:5.2} ms  ({:4.0} fps)  pending ticks: {pending_ticks}",
                frame_ms,
                1000.0 / frame_ms.max(0.01),
            ));
            ui.separator();
            ui.heading("Vehicle");
            ui.label(format!(
                "pos x {:8.1}  y {:8.1}  z {:8.1}",
                t.position.x, t.position.y, t.position.z,
            ));
            ui.label(format!(
                "vel x {:7.2}  y {:7.2}  z {:7.2}",
                t.velocity.x, t.velocity.y, t.velocity.z,
            ));
            ui.label(format!(
                "speed  {:6.2} m/s  ({:5.1} kt)",
                t.airspeed_total,
                t.airspeed_total * 1.944,
            ));
            ui.label(format!(
                "altitude {:7.1} m  ({:5.0} ft)",
                t.altitude_msl,
                t.altitude_msl * 3.281,
            ));
            ui.label(format!(
                "attitude pitch {:6.1}  yaw {:6.1}  roll {:6.1}  (deg)",
                t.pitch_rad.to_degrees(),
                t.heading_rad.to_degrees(),
                t.bank_rad.to_degrees(),
            ));
            ui.separator();
            ui.heading("Controls");
            ui.label(format!("throttle {:5.1}%", t.throttle * 100.0));
            ui.label(format!(
                "pitch {:+.2}  yaw {:+.2}  roll {:+.2}",
                t.pitch_input, t.yaw_input, t.roll_input,
            ));
            ui.label(format!(
                "flaps   {}  ({:2.0}°)",
                t.flap_position.label(),
                t.flap_position.deflection_deg(),
            ));
            // Water rudder state indicator. Green when deployed, red-tinted
            // when retracted. Only meaningful on float variants; wheels
            // variants leave this permanently retracted.
            let (wr_color, wr_state) = if t.water_rudder_deployed {
                (egui::Color32::from_rgb(120, 220, 120), "DEPLOYED")
            } else {
                (egui::Color32::from_rgb(220, 120, 120), "retracted")
            };
            ui.horizontal(|ui| {
                ui.label("water rudder (T)");
                ui.colored_label(wr_color, wr_state);
            });
            ui.separator();
            ui.small("W/S throttle  |  stick: ↑ dive / ↓ climb  |  ←/→ roll  |  Q/E yaw  |  F/C flaps ext/ret  |  T water rudder  |  B brakes  |  1/2/3 view  |  V cycle  |  R reset  |  Esc quit");
        });

    // Top-right: flight-model dynamics breakdown. Same Telemetry source as the left
    // panel -- single-source invariant per DESIGN.md § Telemetry derivation.
    egui::Window::new("dynamics")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
        .default_width(280.0)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.heading("Airspeeds & AoA");
            ui.label(format!(
                "total     {:6.2} m/s  ({:5.1} kt)",
                t.airspeed_total,
                t.airspeed_total * 1.944,
            ));
            ui.label(format!(
                "horiz fwd {:6.2} m/s  ({:5.1} kt)",
                t.airspeed_horizontal_fwd,
                t.airspeed_horizontal_fwd * 1.944,
            ));
            ui.label(format!(
                "α  {:+6.2}°    β  {:+6.2}°",
                t.alpha_rad.to_degrees(),
                t.beta_rad.to_degrees(),
            ));
            ui.label(format!("Cl {:6.3}    Cd {:6.3}", t.cl, t.cd));

            // α-based stall alert (Phase 2.4.4d). Reads the StallState classification
            // that the flight model produces each tick from |α| vs the stall table
            // peak. Handles dives (where airspeed is high but α can still exceed
            // stall) and accelerated / G-loaded stalls (where airspeed is above Vs
            // but high lift demand pushes α past stall) correctly because the test
            // is directly on α.
            ui.separator();
            ui.heading("Stall alert");
            let (label, color) = match t.stall_state {
                StallState::Normal => ("NORMAL", egui::Color32::from_rgb(120, 200, 120)),
                StallState::Approach => ("APPROACH", egui::Color32::from_rgb(240, 200, 80)),
                StallState::Stall => ("STALL", egui::Color32::from_rgb(240, 80, 80)),
            };
            ui.colored_label(
                color,
                format!(
                    "{label}    (α = {:+.1}°; stall at ±16.0°)",
                    t.alpha_rad.to_degrees(),
                ),
            );

            ui.separator();
            ui.heading("Forces (m/s^2)");
            ui.label(format!(
                "thrust   x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                t.thrust.x,
                t.thrust.y,
                t.thrust.z,
                t.thrust.length(),
            ));
            ui.label(format!(
                "lift     x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                t.lift.x,
                t.lift.y,
                t.lift.z,
                t.lift.length(),
            ));
            ui.label(format!(
                "drag     x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                t.drag.x,
                t.drag.y,
                t.drag.z,
                t.drag.length(),
            ));
            ui.label(format!(
                "gravity  x {:+6.2}  y {:+6.2}  z {:+6.2}",
                t.gravity.x, t.gravity.y, t.gravity.z,
            ));
            ui.separator();
            ui.label(format!(
                "total    x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                t.linear_accel.x,
                t.linear_accel.y,
                t.linear_accel.z,
                t.linear_accel.length(),
            ));
            ui.separator();
            ui.heading("Angular (body frame)");
            // Rates in flowplan storage; physics convention in parens for the signs.
            ui.label(format!(
                "rate   p {:+.3}  r {:+.3}  y {:+.3}  (rad/s, flowplan storage)",
                t.actual_angular_vel.x, t.actual_angular_vel.z, t.actual_angular_vel.y,
            ));
            ui.label(format!(
                "accel  p {:+.3}  r {:+.3}  y {:+.3}  (rad/s²)",
                t.angular_accel.x, t.angular_accel.z, t.angular_accel.y,
            ));
            ui.separator();
            ui.heading("Moments (N·m, physics convention)");
            // Physics convention: +M nose up, +L right-wing-down, +N nose right.
            // Columns: total + (control, stability, damping, coupling). The coupling
            // column captures cross-axis effects (2.4.6): P-factor + adverse yaw on
            // the yaw axis, torque reaction on roll.
            ui.label(format!(
                "pitch M  {:+8.0}   ctl {:+6.0}  stab {:+6.0}  damp {:+6.0}  cpl {:+6.0}",
                t.pitch_moment.total(),
                t.pitch_moment.control,
                t.pitch_moment.stability,
                t.pitch_moment.damping,
                t.pitch_moment.coupling,
            ));
            ui.label(format!(
                "roll  L  {:+8.0}   ctl {:+6.0}  stab {:+6.0}  damp {:+6.0}  cpl {:+6.0}",
                t.roll_moment.total(),
                t.roll_moment.control,
                t.roll_moment.stability,
                t.roll_moment.damping,
                t.roll_moment.coupling,
            ));
            ui.label(format!(
                "yaw   N  {:+8.0}   ctl {:+6.0}  stab {:+6.0}  damp {:+6.0}  cpl {:+6.0}",
                t.yaw_moment.total(),
                t.yaw_moment.control,
                t.yaw_moment.stability,
                t.yaw_moment.damping,
                t.yaw_moment.coupling,
            ));
            ui.separator();
            ui.heading("Engine");
            ui.label(format!(
                "power  {:5.1} kW  ({:4.0} hp)   η_prop  {:.2}",
                t.engine_power_kw,
                t.engine_power_kw * 1.341, // kW → hp
                t.prop_efficiency,
            ));
            ui.label(format!(
                "fuel flow {:5.1} gph   remaining {:5.1} gal",
                t.fuel_flow_gph, t.fuel_gal,
            ));
            ui.label(format!(
                "ρ {:.3} kg/m³  ({:3.0}% SL)",
                t.air_density_kg_m3,
                t.density_ratio * 100.0,
            ));
            let endurance_label = if t.endurance_seconds.is_finite() {
                let minutes = t.endurance_seconds / 60.0;
                let hours = minutes / 60.0;
                if hours >= 1.0 {
                    format!("endurance {hours:.2} h at current burn")
                } else {
                    format!("endurance {minutes:.1} min at current burn")
                }
            } else {
                "endurance  -- (idle / no fuel)".to_string()
            };
            ui.label(endurance_label);
            ui.separator();
            if let Some(spec) = aircraft {
                ui.heading("Aircraft");
                ui.label(format!("{} ({:.0} hp)", spec.name, spec.engine.power_hp));
                ui.label(format!(
                    "cruise {:.1} m/s   Vs clean {:.1} m/s   Vso {:.1} m/s",
                    spec.speeds.cruise_mps, spec.speeds.vs_clean_mps, spec.speeds.vso_mps,
                ));
            } else {
                ui.small("(no AircraftSpec loaded -- using placeholder constants)");
            }
        });

    // Bottom-left: glide-slope / flight-path readouts that pair with the attitude
    // indicator widget rendered below this panel. Anchored via `LEFT_BOTTOM` with a
    // vertical offset that lifts the panel above the widget's 180 px + 20 px margin.
    egui::Window::new("glide")
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -210.0))
        .default_width(220.0)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.label(format!(
                "FPA    {:+6.2}°     pitch {:+6.2}°",
                t.flight_path_angle_rad.to_degrees(),
                t.pitch_rad.to_degrees(),
            ));
            ui.label(format!(
                "α      {:+6.2}°     β     {:+6.2}°",
                t.alpha_rad.to_degrees(),
                t.beta_rad.to_degrees(),
            ));
            ui.label(format!(
                "VS   {:+6.2} m/s  ({:+5.0} ft/min)",
                t.vertical_speed_mps,
                t.vertical_speed_mps * 196.85,
            ));
            let ld_label = if t.glide_ratio.is_finite() {
                format!("L/D  {:5.2} : 1", t.glide_ratio)
            } else {
                "L/D    --  (level / climbing)".to_string()
            };
            ui.label(ld_label);
        });
}

/// Per-frame debug-camera HUD inputs.
///
/// Bundled as a reference-struct because passing 10+ args through
/// `render()` is silly.
///
/// Populated by `tick_and_render` from `self.debug_camera`, `self.streaming_*`,
/// and `self.last_*_rebuild_*` — all state already exists for 3.6.3c.5b.
pub struct DebugCameraOverlay<'a> {
    /// World-space camera position.
    pub position: glam::Vec3,
    /// Heading in degrees (0-360, 0 = north / -Z).
    pub heading_deg: f32,
    /// Pitch in degrees (+ up, - down).
    pub pitch_deg: f32,
    /// Current speed tier (m/s).
    pub speed_mps: f32,
    /// Active sub-biome cell count from `StreamingCellSet.active_count()`.
    pub active_cell_count: usize,
    /// Live feature-instance count in the renderer (3.6.3c.5c.1). Reads
    /// `Renderer::instance_count()`. Zero when `FLOWPLAN_FEATURE_INSTANCES=1`
    /// is unset + during the pre-upload frames while bake is in flight.
    pub instance_count: u32,
    /// Most recent terrain mesh-rebuild CPU cost (worker thread).
    pub last_terrain_cpu_ms: f32,
    /// Most recent terrain mesh-rebuild GPU upload cost.
    pub last_terrain_gpu_ms: f32,
    /// True if a worker rebuild is currently in flight.
    pub rebuild_in_flight: bool,
    /// Streaming terrain mesh center (reassuring "yes, the mesh is
    /// under the camera" readout).
    pub terrain_mesh_center: glam::Vec2,
    /// Borrow of the scenario-playback controller states isn't needed
    /// here; lifetime parameter is for future extensions that may
    /// carry borrowed slices (e.g. cell-active-set for display).
    pub _phantom: core::marker::PhantomData<&'a ()>,
}

#[allow(clippy::too_many_lines)]
fn build_debug_camera_ui(ctx: &Context, frame_ms: f32, o: &DebugCameraOverlay<'_>) {
    // Main debug-camera panel: position / heading / speed-tier / FPS /
    // frame time. Anchored top-left; compact enough to fit a 1080p HUD.
    egui::Window::new("debug camera")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(10.0, 10.0))
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "pos   x={:>+9.1}  y={:>+9.1}  z={:>+9.1}",
                o.position.x, o.position.y, o.position.z
            ));
            ui.label(format!(
                "look  heading={:>6.1}°  pitch={:>+6.1}°",
                o.heading_deg, o.pitch_deg
            ));
            ui.label(format!("speed  {:>5.0} m/s   (Tab to cycle)", o.speed_mps));
            ui.separator();
            let fps = if frame_ms > 0.0 {
                1000.0 / frame_ms
            } else {
                0.0
            };
            ui.label(format!("fps   {fps:>6.1}   frame {frame_ms:>5.2} ms"));
            ui.separator();
            ui.label("bindings:  WASD  ⇧LShift /⇧LCtrl  mouse  Tab  ` toggle  ESC release");
        });

    // Streaming-layer panel: mesh center, active-cell count, rebuild
    // status. Useful diagnostic while flying across the world to verify
    // streaming keeps up.
    egui::Window::new("streaming")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "terrain mesh center  ({:>+9.1}, {:>+9.1})",
                o.terrain_mesh_center.x, o.terrain_mesh_center.y
            ));
            let from_center = (o.position.x - o.terrain_mesh_center.x)
                .hypot(o.position.z - o.terrain_mesh_center.y);
            ui.label(format!("distance from mesh center  {from_center:>6.0} m"));
            ui.separator();
            ui.label(format!("active cells  {}", o.active_cell_count));
            ui.label(format!("instances     {}", o.instance_count));
            ui.separator();
            let (status_color, status_text) = if o.rebuild_in_flight {
                (
                    egui::Color32::from_rgb(240, 200, 80),
                    "rebuild in flight".to_string(),
                )
            } else {
                (egui::Color32::from_rgb(120, 220, 120), "idle".to_string())
            };
            ui.colored_label(status_color, format!("status:  {status_text}"));
            ui.label(format!(
                "last rebuild  cpu={:>6.1} ms   gpu={:>5.2} ms",
                o.last_terrain_cpu_ms, o.last_terrain_gpu_ms
            ));
        });
}
