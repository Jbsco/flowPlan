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
use flowplan_sim::{SimSnapshot, SimState};
use glam::EulerRot;
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
        snapshot: &SimSnapshot,
        live_sim: &SimState,
        pending_ticks_last_frame: u32,
    ) {
        let now = Instant::now();
        let dt_ms = now.duration_since(self.last_frame_time).as_secs_f32() * 1000.0;
        self.last_frame_time = now;
        // Exponential moving average (alpha ~ 0.1) for a readable FPS readout.
        self.frame_time_ms_ema = 0.9 * self.frame_time_ms_ema + 0.1 * dt_ms;

        let raw_input = self.state.take_egui_input(window);
        let full_output = self.ctx.run(raw_input, |ctx| {
            build_ui(
                ctx,
                snapshot,
                live_sim,
                self.frame_time_ms_ema,
                pending_ticks_last_frame,
            );
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
fn build_ui(ctx: &Context, snap: &SimSnapshot, live: &SimState, frame_ms: f32, pending_ticks: u32) {
    // glam's `to_euler(YXZ)` returns the angles in the axis order of the enum: (y, x, z).
    let (yaw, pitch, roll) = live.vehicle.orientation.to_euler(EulerRot::YXZ);
    let speed = snap.vehicle.velocity.length();
    let altitude = snap.vehicle.position.y;
    let view_label = match snap.camera.view_kind {
        0 => "First-person",
        1 => "Chase (3P)",
        2 => "Exterior orbit",
        _ => "?",
    };

    egui::Window::new("flowPlan debug")
        .default_pos([10.0, 10.0])
        .default_width(260.0)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.label(format!("tick: {}", snap.tick));
            ui.label(format!(
                "frame: {:5.2} ms  ({:4.0} fps)  pending ticks: {pending_ticks}",
                frame_ms,
                1000.0 / frame_ms.max(0.01),
            ));
            ui.separator();
            ui.label(format!("view: {view_label}"));
            ui.separator();
            ui.heading("Vehicle");
            ui.label(format!(
                "pos x {:8.1}  y {:8.1}  z {:8.1}",
                snap.vehicle.position.x, snap.vehicle.position.y, snap.vehicle.position.z,
            ));
            ui.label(format!(
                "vel x {:7.2}  y {:7.2}  z {:7.2}",
                snap.vehicle.velocity.x, snap.vehicle.velocity.y, snap.vehicle.velocity.z,
            ));
            ui.label(format!("speed  {speed:6.2} m/s  ({:5.1} kt)", speed * 1.944));
            ui.label(format!("altitude {altitude:7.1} m  ({:5.0} ft)", altitude * 3.281));
            ui.label(format!(
                "attitude pitch {:6.1}  yaw {:6.1}  roll {:6.1}  (deg)",
                pitch.to_degrees(),
                yaw.to_degrees(),
                roll.to_degrees(),
            ));
            ui.separator();
            ui.heading("Controls");
            ui.label(format!(
                "throttle {:5.1}%",
                live.vehicle.controls.throttle * 100.0
            ));
            ui.label(format!(
                "pitch {:+.2}  yaw {:+.2}  roll {:+.2}",
                live.vehicle.controls.pitch,
                live.vehicle.controls.yaw,
                live.vehicle.controls.roll,
            ));
            ui.separator();
            ui.small("W/S throttle  |  stick: ↑ dive / ↓ climb  |  ←/→ roll  |  Q/E yaw  |  1/2/3 view  |  V cycle  |  R reset  |  Esc quit");
        });

    // Top-right: flight-model dynamics breakdown. Shows the current force
    // composition, airspeeds, and commanded vs actual angular velocity.
    let f = &live.last_flight;
    egui::Window::new("flowPlan dynamics")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
        .default_width(280.0)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.heading("Airspeeds");
            ui.label(format!(
                "total     {:6.2} m/s  ({:5.1} kt)",
                f.airspeed,
                f.airspeed * 1.944,
            ));
            ui.label(format!(
                "horiz fwd {:6.2} m/s  ({:5.1} kt)",
                f.horizontal_airspeed,
                f.horizontal_airspeed * 1.944,
            ));

            // Stall alert (pre-2.4.4 heuristic). Compares horizontal forward airspeed
            // to the aircraft's published Vs (clean stall speed). Three tiers:
            // NORMAL >= 1.2 Vs, APPROACH 1.0..1.2 Vs, STALL below Vs. Real
            // angle-of-attack-based stall modeling arrives with Milestone 2.4.4.
            if let Some(spec) = &live.aircraft {
                ui.separator();
                ui.heading("Stall alert");
                let vs = spec.speeds.vs_clean_mps.max(1e-3);
                let ratio = f.horizontal_airspeed / vs;
                let (label, color) = if ratio >= 1.2 {
                    ("NORMAL", egui::Color32::from_rgb(120, 200, 120))
                } else if ratio >= 1.0 {
                    ("APPROACH", egui::Color32::from_rgb(240, 200, 80))
                } else {
                    ("STALL", egui::Color32::from_rgb(240, 80, 80))
                };
                ui.colored_label(
                    color,
                    format!(
                        "{label}    (airspeed / Vs = {:.2};  Vs = {:.1} m/s / {:.0} kt)",
                        ratio,
                        vs,
                        vs * 1.944,
                    ),
                );
            }

            ui.separator();
            ui.heading("Forces (m/s^2)");
            ui.label(format!(
                "thrust   x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                f.thrust.x,
                f.thrust.y,
                f.thrust.z,
                f.thrust.length(),
            ));
            ui.label(format!(
                "lift     x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                f.lift.x,
                f.lift.y,
                f.lift.z,
                f.lift.length(),
            ));
            ui.label(format!(
                "drag     x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                f.drag.x,
                f.drag.y,
                f.drag.z,
                f.drag.length(),
            ));
            ui.label(format!(
                "gravity  x {:+6.2}  y {:+6.2}  z {:+6.2}",
                f.gravity.x, f.gravity.y, f.gravity.z,
            ));
            ui.separator();
            ui.label(format!(
                "total    x {:+6.2}  y {:+6.2}  z {:+6.2}  (|{:5.2}|)",
                f.linear_accel.x,
                f.linear_accel.y,
                f.linear_accel.z,
                f.linear_accel.length(),
            ));
            ui.separator();
            ui.heading("Angular (rad/s, body frame)");
            ui.label(format!(
                "commanded  p {:+.3}  r {:+.3}  y {:+.3}",
                f.commanded_angular_vel.x, f.commanded_angular_vel.z, f.commanded_angular_vel.y,
            ));
            ui.label(format!(
                "actual     p {:+.3}  r {:+.3}  y {:+.3}",
                live.vehicle.angular_velocity.x,
                live.vehicle.angular_velocity.z,
                live.vehicle.angular_velocity.y,
            ));
            ui.label(format!("response rate  {:.2} / s", f.ang_response_rate));
            ui.separator();
            if let Some(spec) = &live.aircraft {
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
}
