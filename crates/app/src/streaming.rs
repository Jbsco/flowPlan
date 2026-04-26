//! Render streaming — uniform terrain mesh + Voronoi cell active set
//! (Phase 3.6.3c.5b).
//!
//! Two-layer hybrid streaming for the 5000 km world. See memory
//! `streaming_render_architecture` + MAPGEN § Render streaming for
//! the pinned design.
//!
//! - **Layer 1**: [`StreamingTerrain`] + [`StreamingWater`] — uniform
//!   meshes that recenter on the active camera when it moves more
//!   than [`REBUILD_THRESHOLD_M`] from the current center.
//! - **Layer 2**: [`StreamingCellSet`] — Voronoi cells within
//!   [`STREAM_RADIUS_M`] of the active camera; refreshed per frame.
//!
//! Active-camera arbitration goes through [`ActiveCamera`] — single
//! source of truth for both streaming layers + renderer's
//! `CameraUniform`.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use std::sync::Arc;

use flowplan_assets::CpuMesh;
use flowplan_world::composed::WorldRefsArc;
use flowplan_world::terrain::{HeightSampler, Terrain};
use flowplan_world::voronoi_cells::{CellId, SubBiomeCellCatalog};
use glam::Vec2;

/// Source the streaming-terrain mesh samples for vertex height. Two
/// variants:
///
/// - **`Terrain`**: L1 only (continental + biome topography). Cheap —
///   single fbm chain per vertex, no per-vertex Voronoi-cell lookup.
///   Used in fast-iteration mode (no `FLOWPLAN_FEATURE_INSTANCES`).
/// - **`Composed`**: L1 + L3 (feature deltas). Adds ~50-100 ns / vertex
///   (a `nearest_cell` lookup + delta dispatch) on top of L1, so the
///   ~1.4 M-vertex disc rebuild grows by ~70-140 ms (~5-10 % of the
///   pre-existing 1.5-2 s rebuild). Visible cost; required to render
///   lake-cluster bowls, mesa plateaus, gas-station pads, etc.
///
/// Cloning is cheap — both variants are `Arc`-backed.
///
/// Phase 3.6.3c.4: the `Composed` variant is preserved for the
/// legacy single-mesh `StreamingTerrain` path; chunked terrain now
/// consumes `WorldRefsArc` directly. `dead_code` allow keeps the
/// variant available for fast-iteration paths without forcing the
/// chunked manager.
#[derive(Clone)]
#[allow(dead_code)]
pub enum HeightSource {
    /// L1-only sampler (no L3 deltas).
    Terrain(Arc<Terrain>),
    /// L1+L3 composed sampler (feature deltas applied).
    Composed(WorldRefsArc),
}

impl HeightSource {
    fn height(&self, x: f32, z: f32) -> f32 {
        match self {
            Self::Terrain(t) => t.height(x, z),
            Self::Composed(refs) => refs.height_at(x, z),
        }
    }
}

/// Streaming terrain disc radius (m). Terrain mesh is a circular disc
/// of this radius, rendered in a rectangular bounding-box grid with
/// out-of-disc triangles dropped. 20 km radius gives consistent
/// horizon distance in all directions (square mesh would show 14 km
/// on axes vs 20 km at corners, visibly inconsistent).
///
/// Performance-slider candidate for Phase 4.8 settings UI.
pub const TERRAIN_DISC_RADIUS_M: f32 = 20_000.0;

/// Streaming terrain half-extent (m) = disc radius; the bounding-box
/// grid spans 2× this on a side (40 km × 40 km before disc-clip).
pub const TERRAIN_HALF_EXTENT_M: f32 = TERRAIN_DISC_RADIUS_M;

/// Cells per side on the bounding-box rectangular grid. 1333 ≈ 30 m
/// per cell at the 20 km disc radius. Disc-clip drops ~22% of cells
/// (outside circle) → ~1.4 M effective vertices. Async worker rebuild
/// at this size measures ~1.5-2 s per user direction.
pub const TERRAIN_CELLS: u32 = 1333;

/// Water disc radius. Matches terrain for consistent horizon.
pub const WATER_DISC_RADIUS_M: f32 = 20_000.0;

/// Water half-extent = disc radius.
pub const WATER_HALF_EXTENT_M: f32 = WATER_DISC_RADIUS_M;

/// Water cells per side on the bounding-box grid. Coarser than terrain
/// — water is flat in 3.6.3c.5b (FFT ocean displacement later).
/// 400 ≈ 100 m per cell at 20 km.
pub const WATER_CELLS: u32 = 400;

/// Camera-movement threshold that triggers a mesh rebuild.
///
/// Default 5 km gives substantial room inside the 20 km disc before
/// the edge approaches. At DHC-2 cruise (~60 m/s) = rebuild every
/// ~83 sec. At 500 m/s debug-cam = every ~10 sec. Async worker
/// finalizes on a later frame regardless of player motion, so rebuild
/// frequency is a budget decision not a responsiveness one.
pub const REBUILD_THRESHOLD_M: f32 = 5_000.0;

/// Large-jump threshold (Phase 3.6.3c.5d follow-up). When the camera
/// moves more than this distance from the current mesh center in a
/// single frame, `maybe_rebuild` synchronously re-bakes the mesh
/// instead of kicking off an async worker. Reasoning: beyond
/// [`TERRAIN_HALF_EXTENT_M`] the old mesh is entirely outside the
/// view frustum, so the async worker's "continue rendering the old
/// mesh while the new one bakes" story yields ~1 s of empty scene.
/// Sync rebuild blocks the render loop once (~1 s) but skips the
/// empty-scene artifact.
///
/// Normal play never crosses this threshold in one frame:
/// - Cruise (~100 m/s): 1.7 m/frame
/// - Debug top-tier (5 km/s): 83 m/frame
///
/// Only discrete camera jumps (spawn-near at init, debug ↔ sim
/// toggle across >20 km, future teleport features) trigger the sync
/// path.
pub const LARGE_JUMP_THRESHOLD_M: f32 = TERRAIN_HALF_EXTENT_M;

/// Voronoi-cell streaming radius. Cells whose seed position is within
/// this distance of the active camera are in the active set; others
/// are not. Used by Layer 2 content streaming (feature instances,
/// future LOD, hub markers, weather overlays, audio sources).
///
/// Sized as `TERRAIN_DISC_RADIUS_M + cell-extent cushion`. A cell's
/// polygon can extend inward from its seed by up to ~half the maximum
/// neighbor-spacing (Bridson observed max ~3500 m → polygon radius ~1750 m).
/// Cells whose seed sits just outside the terrain disc but whose polygon
/// overlaps the visible disc would otherwise leave un-streamed gaps —
/// "Streaming-radius square-vs-circle gap" (Task #261, .12k). The 2 km
/// cushion covers the observed worst-case polygon extension with margin.
///
/// Cost: cell count scales as r². At 22 km vs 20 km that's +21% cells in
/// the active set; per-frame `cells_within_radius` stays well under 1 ms.
pub const STREAM_RADIUS_M: f32 = 22_000.0;

/// Source-of-truth for the camera pose used by both streaming layers
/// and the renderer.
///
/// In normal play: returns the sim camera (Chase / Cockpit / Orbit per
/// `sim::camera::ViewKind`). In debug mode (3.6.3c.5a backtick toggle):
/// returns the debug-fly camera. Caller (game loop) sets `mode`; both
/// streaming layers + renderer query this struct for the current pose.
///
/// `orientation` / `fov_y_radians` / `debug_active` are read by 3.6.3c.5a
/// (debug-camera render path + HUD gating) when it lands. 3.6.3c.5b only
/// needs `position` for streaming recenter; tests exercise the full
/// struct. Inline allow per `CODING_RULES.md` §61 — scoped, milestone-
/// annotated, follow-up consumer is queued on Task #223.
#[derive(Clone, Copy, Debug)]
pub struct ActiveCamera {
    /// Current camera position in world coordinates.
    pub position: glam::Vec3,
    /// Current camera orientation (used by renderer; streaming only
    /// uses position). Consumed in 3.6.3c.5a.
    pub orientation: glam::Quat,
    /// Field of view (used by renderer). Consumed in 3.6.3c.5a.
    pub fov_y_radians: f32,
    /// True when the debug-fly camera is active (3.6.3c.5a). Streaming
    /// behavior is identical regardless; the flag is for HUD + input
    /// gating in the debug-camera milestone.
    pub debug_active: bool,
}

impl ActiveCamera {
    /// Build from a sim `CameraSnapshot` (the normal-play case — the
    /// snapshot is what the app reads from the sim-render handoff).
    #[must_use]
    pub fn from_sim(cam: flowplan_sim::CameraSnapshot) -> Self {
        Self {
            position: cam.position,
            orientation: cam.orientation,
            fov_y_radians: cam.fov_y_radians,
            debug_active: false,
        }
    }

    /// Position projected to the XZ plane — what the streaming layers
    /// care about.
    #[must_use]
    pub fn position_xz(&self) -> Vec2 {
        Vec2::new(self.position.x, self.position.z)
    }
}

/// Completed-rebuild message sent from the worker thread back to the
/// main thread via `mpsc::channel`. Carries the freshly-built mesh + its
/// new center + the wall-clock CPU cost for instrumentation.
struct FinishedRebuild {
    mesh: CpuMesh,
    center: Vec2,
    cpu_ms: f32,
}

/// Layer 1 — uniform terrain mesh that recenters on the active camera.
///
/// Holds the current `CpuMesh` (CPU-side data); GPU buffer ownership
/// is the renderer's job. After rebuild, the renderer's terrain
/// `GpuMesh` must be updated via `Renderer::update_terrain`
/// (caller's responsibility).
///
/// **Async rebuild (3.6.3c.5b-followup, 2026-04-23)**: mesh builds
/// happen on a worker thread to avoid blocking the render loop. When
/// the camera crosses [`REBUILD_THRESHOLD_M`], a worker is spawned
/// holding a cloned `Terrain`; the main thread continues rendering the
/// current mesh. On each frame, [`Self::maybe_rebuild`] first tries to
/// finalize any in-flight worker (non-blocking `try_recv`); then
/// decides whether to kick off a new rebuild. The 1100 ms-ish
/// `terrain.height`-dominated cost becomes invisible to the player —
/// hitch removed per profiling data from canonical fly-test 2026-04-23.
pub struct StreamingTerrain {
    cpu_mesh: CpuMesh,
    center: Vec2,
    half_extent: f32,
    resolution: u32,
    /// Sampler used for vertex height. `HeightSource::Terrain` for the
    /// fast L1-only path; `HeightSource::Composed` to render L3 feature
    /// deltas. Cloned per worker spawn — clones are `Arc` bumps.
    height_source: HeightSource,
    /// True iff `maybe_rebuild` finalized a worker rebuild on the most
    /// recent call (i.e. mesh was swapped). Caller checks this to
    /// decide whether to re-upload to GPU.
    just_rebuilt: bool,
    /// Last finalized rebuild's worker-measured CPU time (ms). Exposed
    /// for instrumentation / HUD readout.
    last_rebuild_cpu_ms: f32,
    /// Set when a worker rebuild is in-flight; cleared when the worker
    /// sends its result back (or when we observe disconnection).
    pending: Option<std::sync::mpsc::Receiver<FinishedRebuild>>,
    /// Center-position the in-flight worker targets. If the camera
    /// moves to a materially-different position while the worker is
    /// mid-build, we DON'T kick off another rebuild (only one in flight
    /// at a time); the next frame's `maybe_rebuild` will chain a fresh
    /// one once the current finishes.
    pending_center: Option<Vec2>,
}

impl StreamingTerrain {
    /// Initial bake — synchronous on the main thread so the first
    /// frame has a mesh to render. Builds a disc-shaped mesh of
    /// radius = `half_extent` (3.6.3c.5b-followup: circular load zone).
    ///
    /// `height_source` selects L1 vs L1+L3. See [`HeightSource`].
    #[must_use]
    pub fn new(
        half_extent: f32,
        resolution: u32,
        initial_center: Vec2,
        height_source: HeightSource,
    ) -> Self {
        let cpu_mesh = CpuMesh::terrain_patch_disc_at(
            resolution,
            half_extent,
            half_extent, // disc_radius == half_extent (circle inscribed in square)
            (initial_center.x, initial_center.y),
            |x, z| height_source.height(x, z),
        );
        Self {
            cpu_mesh,
            center: initial_center,
            half_extent,
            resolution,
            height_source,
            just_rebuilt: true,
            last_rebuild_cpu_ms: 0.0,
            pending: None,
            pending_center: None,
        }
    }

    /// Per-frame entry point.
    ///
    /// **Finalize phase**: non-blocking `try_recv` on any in-flight
    /// worker. If the worker has finished, swap its mesh in, set
    /// `just_rebuilt = true`, clear pending state.
    ///
    /// **Kick-off phase**: if no worker is in flight AND camera has
    /// moved past [`REBUILD_THRESHOLD_M`] from current center, spawn
    /// a new worker with a cloned `HeightSource` (Arc bump).
    ///
    /// Main thread work per frame is effectively free — a channel probe
    /// plus a spawn decision. The 1 s-ish mesh-build cost lives on the
    /// worker thread, invisible to the render loop.
    pub fn maybe_rebuild(&mut self, camera_pos_xz: Vec2) {
        self.just_rebuilt = false;

        // Large-jump path — if the camera has moved more than the mesh
        // radius, the current mesh is outside the view frustum and the
        // async "render old mesh while new one bakes" story yields ~1 s
        // of empty scene. Block any in-flight worker, then sync-rebuild
        // here. One-time ~1 s hitch is acceptable because large jumps
        // only happen on discrete user actions (debug ↔ sim toggle,
        // spawn-near at init, future teleport).
        if camera_pos_xz.distance(self.center) > LARGE_JUMP_THRESHOLD_M {
            self.sync_rebuild(camera_pos_xz);
            return;
        }

        // Phase 1 — finalize any in-flight worker.
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.cpu_mesh = result.mesh;
                    self.center = result.center;
                    self.last_rebuild_cpu_ms = result.cpu_ms;
                    self.just_rebuilt = true;
                    self.pending = None;
                    self.pending_center = None;
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Worker still running; don't start another.
                },
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Worker panicked or dropped the sender. Clear
                    // state so a future rebuild can kick off.
                    tracing::warn!(
                        target: "streaming",
                        "terrain rebuild worker disconnected without sending result"
                    );
                    self.pending = None;
                    self.pending_center = None;
                },
            }
        }

        // Phase 2 — kick off a new rebuild if needed.
        if self.pending.is_none() && camera_pos_xz.distance(self.center) > REBUILD_THRESHOLD_M {
            let (tx, rx) = std::sync::mpsc::channel();
            let height_source = self.height_source.clone();
            let resolution = self.resolution;
            let half_extent = self.half_extent;
            let target_center = camera_pos_xz;
            let spawn_result = std::thread::Builder::new()
                .name("streaming-terrain-build".into())
                .spawn(move || {
                    let t0 = std::time::Instant::now();
                    let mesh = CpuMesh::terrain_patch_disc_at(
                        resolution,
                        half_extent,
                        half_extent, // disc_radius
                        (target_center.x, target_center.y),
                        |x, z| height_source.height(x, z),
                    );
                    let cpu_ms = t0.elapsed().as_secs_f32() * 1000.0;
                    // Receiver dropped ⇒ main thread gave up on us
                    // (e.g. app closing). Explicit drop of the
                    // `Result` is the documented "fire and forget"
                    // pattern; clippy's let_underscore_drop lint
                    // flags the shorthand `let _ = ...`.
                    drop(tx.send(FinishedRebuild {
                        mesh,
                        center: target_center,
                        cpu_ms,
                    }));
                });

            // If the OS couldn't spawn the thread (ulimit, exhausted
            // stack reservation, etc.), log + stay on the current mesh.
            // We're NOT `.expect()`-ing per CODING_RULES — spawn failure
            // is recoverable (we'll just render the stale mesh and try
            // again next frame after clearing pending).
            match spawn_result {
                Ok(_handle) => {
                    self.pending = Some(rx);
                    self.pending_center = Some(target_center);
                },
                Err(e) => {
                    tracing::error!(
                        target: "streaming",
                        error = %e,
                        "failed to spawn terrain-build worker; continuing with stale mesh"
                    );
                },
            }
        }
    }

    /// Synchronous rebuild at a new center. Blocks the caller for the
    /// full bake duration (~1 s). Any in-flight worker is dropped (its
    /// result is no longer needed — we're committing to a fresh bake
    /// at a different position).
    ///
    /// Invoked by [`Self::maybe_rebuild`] when the camera jumps further
    /// than [`LARGE_JUMP_THRESHOLD_M`] in one frame.
    fn sync_rebuild(&mut self, camera_pos_xz: Vec2) {
        // Drop in-flight worker; its result targets the now-obsolete
        // center and would only be written back as a stale mesh.
        self.pending = None;
        self.pending_center = None;

        let t0 = std::time::Instant::now();
        self.cpu_mesh = CpuMesh::terrain_patch_disc_at(
            self.resolution,
            self.half_extent,
            self.half_extent,
            (camera_pos_xz.x, camera_pos_xz.y),
            |x, z| self.height_source.height(x, z),
        );
        self.center = camera_pos_xz;
        self.last_rebuild_cpu_ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.just_rebuilt = true;
        tracing::info!(
            target: "streaming",
            cpu_ms = self.last_rebuild_cpu_ms,
            center_x = camera_pos_xz.x,
            center_z = camera_pos_xz.y,
            "terrain sync rebuild (large camera jump)"
        );
    }

    /// True iff the most recent `maybe_rebuild` call swapped in a
    /// worker-built mesh. Caller should re-upload to GPU when this
    /// returns true.
    #[must_use]
    pub const fn just_rebuilt(&self) -> bool {
        self.just_rebuilt
    }

    /// Worker-measured CPU time (ms) of the most recent rebuild.
    /// 0 if no rebuild has completed yet.
    #[must_use]
    pub const fn last_rebuild_cpu_ms(&self) -> f32 {
        self.last_rebuild_cpu_ms
    }

    /// True iff a worker rebuild is currently in flight. Useful for
    /// HUD status ("rebuilding...") + debugging.
    #[must_use]
    pub const fn rebuild_in_flight(&self) -> bool {
        self.pending.is_some()
    }

    /// Borrow the current CPU mesh (for GPU upload).
    #[must_use]
    pub const fn cpu_mesh(&self) -> &CpuMesh {
        &self.cpu_mesh
    }

    /// Current mesh center. Read by 3.6.3c.5a debug HUD (shows "mesh
    /// center at X, Y" for streaming diagnostics). Allow per `CODING_RULES`
    /// §61 — consumer queued on Task #223.
    #[must_use]
    pub const fn center(&self) -> Vec2 {
        self.center
    }
}

/// Layer 1 — uniform water mesh that recenters on the active camera.
///
/// Same pattern as [`StreamingTerrain`] but for the flat water plane.
/// Tessellation is coarser (water is currently flat — 3.5b FFT ocean
/// will add per-vertex displacement later per `weather_water_fft_second_pass`).
pub struct StreamingWater {
    cpu_mesh: CpuMesh,
    center: Vec2,
    half_extent: f32,
    resolution: u32,
    color: [f32; 4],
    just_rebuilt: bool,
}

impl StreamingWater {
    /// Initial bake at the given center. Builds a disc-shaped water
    /// mesh of radius = `half_extent` for consistent-horizon rendering
    /// (3.6.3c.5b-followup).
    #[must_use]
    pub fn new(half_extent: f32, resolution: u32, initial_center: Vec2, color: [f32; 4]) -> Self {
        let cpu_mesh = CpuMesh::water_grid_disc_at(
            resolution,
            half_extent,
            half_extent, // disc_radius == half_extent
            (initial_center.x, initial_center.y),
            color,
        );
        Self {
            cpu_mesh,
            center: initial_center,
            half_extent,
            resolution,
            color,
            just_rebuilt: true,
        }
    }

    /// Rebuild if camera has moved more than [`REBUILD_THRESHOLD_M`].
    pub fn maybe_rebuild(&mut self, camera_pos_xz: Vec2) {
        if camera_pos_xz.distance(self.center) > REBUILD_THRESHOLD_M {
            self.cpu_mesh = CpuMesh::water_grid_disc_at(
                self.resolution,
                self.half_extent,
                self.half_extent,
                (camera_pos_xz.x, camera_pos_xz.y),
                self.color,
            );
            self.center = camera_pos_xz;
            self.just_rebuilt = true;
        } else {
            self.just_rebuilt = false;
        }
    }

    #[must_use]
    pub const fn just_rebuilt(&self) -> bool {
        self.just_rebuilt
    }

    #[must_use]
    pub const fn cpu_mesh(&self) -> &CpuMesh {
        &self.cpu_mesh
    }

    /// Current mesh center. Read by 3.6.3c.5a debug HUD. Allow per
    /// `CODING_RULES` §61 — consumer queued on Task #223.
    #[must_use]
    pub const fn center(&self) -> Vec2 {
        self.center
    }
}

/// Layer 2 — Voronoi cell active set.
///
/// Tracks which sub-biome Voronoi cells are within [`STREAM_RADIUS_M`]
/// of the active camera. Refreshed per frame via the underlying spatial-
/// hash query on `SubBiomeCellCatalog::cells_within_radius`.
///
/// Provides `entered` / `exited` deltas vs. the previous frame for
/// downstream content systems (feature instance streaming, future LOD,
/// hub markers, weather overlays, audio sources) to allocate / free
/// per-cell resources without re-iterating the full active set.
pub struct StreamingCellSet {
    /// Sorted ascending. Source-of-truth for "what's currently active."
    active: Vec<CellId>,
    /// Cells that entered the active set on the most recent refresh.
    /// Caller-consumable on the same frame; cleared at next refresh.
    entered: Vec<CellId>,
    /// Cells that exited the active set on the most recent refresh.
    exited: Vec<CellId>,
    /// Last camera position the set was refreshed for.
    last_camera_pos: Vec2,
    /// Streaming radius (m) used for the most recent refresh.
    radius_m: f32,
}

impl StreamingCellSet {
    /// Initial empty set. Call [`Self::refresh`] before use.
    #[must_use]
    pub fn new(radius_m: f32) -> Self {
        Self {
            active: Vec::new(),
            entered: Vec::new(),
            exited: Vec::new(),
            last_camera_pos: Vec2::ZERO,
            radius_m,
        }
    }

    /// Refresh the active set from the current camera position. Computes
    /// `entered` + `exited` deltas vs. the previous active set.
    ///
    /// O(K) where K is the cell count in radius (~440 at the production
    /// 22 km radius / 2 km cell spacing).
    pub fn refresh(&mut self, camera_pos_xz: Vec2, catalog: &SubBiomeCellCatalog) {
        let new_active =
            catalog.cells_within_radius(camera_pos_xz.x, camera_pos_xz.y, self.radius_m);
        // Both lists are sorted ascending; merge-walk for entered/exited.
        self.entered.clear();
        self.exited.clear();
        let mut i = 0;
        let mut j = 0;
        while i < self.active.len() && j < new_active.len() {
            match self.active[i].cmp(&new_active[j]) {
                std::cmp::Ordering::Less => {
                    self.exited.push(self.active[i]);
                    i += 1;
                },
                std::cmp::Ordering::Greater => {
                    self.entered.push(new_active[j]);
                    j += 1;
                },
                std::cmp::Ordering::Equal => {
                    i += 1;
                    j += 1;
                },
            }
        }
        while i < self.active.len() {
            self.exited.push(self.active[i]);
            i += 1;
        }
        while j < new_active.len() {
            self.entered.push(new_active[j]);
            j += 1;
        }
        self.active = new_active;
        self.last_camera_pos = camera_pos_xz;
    }

    /// Currently-active cells (sorted ascending). Consumed by
    /// feature-instance upload (3.6.3c.5c.1) + debug HUD (shows count).
    #[must_use]
    pub fn active(&self) -> &[CellId] {
        &self.active
    }

    /// Cells that entered on the most recent refresh. Consumed by
    /// 3.6.3c.5c.1 to detect active-set changes (triggers instance
    /// buffer rebuild). Per-cell resource allocation hooks (spawn
    /// audio sources, allocate per-cell GPU buffers) land in 5c.2+.
    #[must_use]
    pub fn entered(&self) -> &[CellId] {
        &self.entered
    }

    /// Cells that exited on the most recent refresh. Consumed by
    /// 3.6.3c.5c.1 to detect active-set changes. Per-cell release
    /// hooks (free audio sources, deallocate buffers) land in 5c.2+.
    #[must_use]
    pub fn exited(&self) -> &[CellId] {
        &self.exited
    }

    /// Number of active cells. Consumed by 3.6.3c.5a debug HUD.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use flowplan_sim::{Camera as SimCamera, CameraSnapshot};
    use flowplan_world::{SubBiomeCellCatalog, Terrain};
    use glam::{Quat, Vec3};

    const TEST_SEED: u64 = 0xF10_1A11;
    const SMALL_EXTENT: f32 = 500.0;
    const SMALL_CELLS: u32 = 32;

    #[test]
    fn rebuild_threshold_smaller_than_half_extent() {
        // The threshold must leave room inside the mesh for the camera
        // to travel. If REBUILD_THRESHOLD_M >= TERRAIN_HALF_EXTENT_M,
        // the mesh edge would be visible after one threshold trip.
        // `const` assertion to avoid the "assertions on constants" lint
        // — forces compile-time evaluation.
        const _: () = assert!(REBUILD_THRESHOLD_M < TERRAIN_HALF_EXTENT_M);
    }

    #[test]
    fn stream_radius_reasonable_default() {
        // 20 km radius disc × 2 km Voronoi cell spacing ≈ 314 active cells.
        // Bounds here are just regression guards on the order-of-magnitude.
        const _: () = assert!(STREAM_RADIUS_M >= 5_000.0);
        const _: () = assert!(STREAM_RADIUS_M <= 50_000.0);
    }

    #[test]
    fn active_camera_from_sim_snapshot() {
        // Construct a sim camera, snapshot it, build ActiveCamera — all
        // fields must round-trip intact.
        let sim_cam = SimCamera {
            position: Vec3::new(100.0, 50.0, -200.0),
            orientation: Quat::from_rotation_y(0.5),
            fov_y_radians: 1.2,
            ..SimCamera::default()
        };
        let snap = CameraSnapshot::from_state(&sim_cam);
        let active = ActiveCamera::from_sim(snap);
        assert_eq!(active.position, sim_cam.position);
        assert_eq!(active.orientation, sim_cam.orientation);
        let eps = 1e-6_f32;
        assert!((active.fov_y_radians - sim_cam.fov_y_radians).abs() < eps);
        assert!(!active.debug_active);
        let xz = active.position_xz();
        assert!((xz.x - 100.0).abs() < eps);
        assert!((xz.y - -200.0).abs() < eps);
    }

    #[test]
    fn streaming_terrain_rebuilds_at_threshold() {
        // Small terrain + small threshold scope so the test is fast.
        // Async rebuild: `maybe_rebuild` kicks off a worker; we poll
        // subsequent frames until finalize.
        let terrain = Terrain::from_seed(TEST_SEED);
        let height_source = HeightSource::Terrain(Arc::new(terrain));
        let mut st =
            StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, glam::Vec2::ZERO, height_source);
        assert!(st.just_rebuilt(), "initial (sync) build is a rebuild");
        assert_eq!(st.center(), glam::Vec2::ZERO);

        // Movement below threshold should NOT kick off a rebuild.
        st.maybe_rebuild(glam::Vec2::new(REBUILD_THRESHOLD_M * 0.5, 0.0));
        assert!(!st.just_rebuilt());
        assert!(!st.rebuild_in_flight());
        assert_eq!(st.center(), glam::Vec2::ZERO, "center unchanged");

        // Movement past threshold kicks off a worker. Poll until
        // finalize swaps the mesh in.
        let new_pos = glam::Vec2::new(REBUILD_THRESHOLD_M * 1.5, 0.0);
        st.maybe_rebuild(new_pos);
        assert!(
            st.rebuild_in_flight() || st.just_rebuilt(),
            "expected worker in flight or immediate finalize"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            st.maybe_rebuild(new_pos);
            if st.just_rebuilt() {
                break;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "worker never finalized — deadline exceeded"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(st.center(), new_pos, "center tracked after finalize");
        assert!(st.last_rebuild_cpu_ms() > 0.0, "worker reported cpu time");
    }

    #[test]
    fn streaming_terrain_large_jump_is_sync() {
        // Camera jumping more than LARGE_JUMP_THRESHOLD_M forces a
        // synchronous rebuild — `just_rebuilt` is true on return, no
        // worker is left in flight. Prevents empty-scene artifact
        // after debug ↔ sim toggle across hundreds of km.
        let terrain = Terrain::from_seed(TEST_SEED);
        let height_source = HeightSource::Terrain(Arc::new(terrain));
        let mut st =
            StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, glam::Vec2::ZERO, height_source);
        assert!(st.just_rebuilt(), "initial bake");

        // First, start an async rebuild at a short distance (so there's
        // something to drop when the big jump arrives).
        st.maybe_rebuild(glam::Vec2::new(REBUILD_THRESHOLD_M * 1.5, 0.0));
        assert!(
            st.rebuild_in_flight() || st.just_rebuilt(),
            "async worker spawned (or immediate finalize)"
        );

        // Now jump more than LARGE_JUMP_THRESHOLD_M. Rebuild must be
        // synchronous: just_rebuilt true, no pending worker, center
        // matches the new position, all on a single call.
        let jump_pos = glam::Vec2::new(LARGE_JUMP_THRESHOLD_M * 2.0, 0.0);
        st.maybe_rebuild(jump_pos);
        assert!(st.just_rebuilt(), "sync rebuild completed");
        assert!(
            !st.rebuild_in_flight(),
            "no pending worker left over from earlier async rebuild"
        );
        assert_eq!(st.center(), jump_pos, "center matches jump target");
    }

    #[test]
    fn streaming_terrain_at_rest_is_deterministic() {
        // Same camera position → same mesh (byte-identical vertex data).
        let terrain = Terrain::from_seed(TEST_SEED);
        let center = glam::Vec2::new(12_345.0, -6_789.0);
        // Share one Arc<Terrain> across both StreamingTerrain instances —
        // the determinism test is about the same input → same output.
        let arc_terrain = Arc::new(terrain);
        let hs_a = HeightSource::Terrain(Arc::clone(&arc_terrain));
        let hs_b = HeightSource::Terrain(Arc::clone(&arc_terrain));
        let a = StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, center, hs_a);
        let b = StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, center, hs_b);
        assert_eq!(a.cpu_mesh().vertices.len(), b.cpu_mesh().vertices.len());
        let eps = 1e-6_f32;
        for (va, vb) in a
            .cpu_mesh()
            .vertices
            .iter()
            .zip(b.cpu_mesh().vertices.iter())
        {
            for i in 0..3 {
                assert!((va.position[i] - vb.position[i]).abs() < eps);
            }
        }
    }

    #[test]
    fn streaming_water_rebuilds_at_threshold() {
        let color = [0.3, 0.5, 0.8, 1.0];
        let mut sw = StreamingWater::new(SMALL_EXTENT, SMALL_CELLS, glam::Vec2::ZERO, color);
        assert!(sw.just_rebuilt());
        assert_eq!(sw.center(), glam::Vec2::ZERO);

        sw.maybe_rebuild(glam::Vec2::new(REBUILD_THRESHOLD_M * 0.5, 0.0));
        assert!(!sw.just_rebuilt());

        let new_pos = glam::Vec2::new(REBUILD_THRESHOLD_M * 1.5, 0.0);
        sw.maybe_rebuild(new_pos);
        assert!(sw.just_rebuilt());
        assert_eq!(sw.center(), new_pos);
    }

    #[test]
    fn mesh_resolution_within_budget() {
        // Default 10 km half-extent × 1000 cells × 4 floats per vertex
        // position + normal + uv + color = ~128 bytes × (1001²) = ~128 MB.
        // We use u32 for color so smaller. Conservative assertion:
        // vertex count stays reasonable (no accidental 10000-cell typo).
        const _: () = assert!(TERRAIN_CELLS <= 2000);
        const _: () = assert!(WATER_CELLS <= 500);
    }

    #[test]
    fn streaming_terrain_handles_world_edge_position() {
        // Camera near the world edge (±2500 km in the 5000 km toroidal
        // world). Mesh build must complete without panic; terrain.height
        // samples wrap toroidally per flowplan_core::wrap_coord.
        let terrain = Terrain::from_seed(TEST_SEED);
        let edge = glam::Vec2::new(2_400_000.0, -2_400_000.0); // near corner
        let st = StreamingTerrain::new(
            SMALL_EXTENT,
            SMALL_CELLS,
            edge,
            HeightSource::Terrain(Arc::new(terrain)),
        );
        assert!(st.just_rebuilt());
        // All vertices finite (no NaN/inf from wrapped sampling).
        for v in &st.cpu_mesh().vertices {
            assert!(v.position[0].is_finite());
            assert!(v.position[1].is_finite());
            assert!(v.position[2].is_finite());
        }
    }

    #[test]
    #[ignore = "expensive: bakes 3.9 M cells; opt-in via --ignored"]
    fn streaming_cell_set_at_canonical_seed() {
        let catalog = SubBiomeCellCatalog::bake(TEST_SEED);
        let mut set = StreamingCellSet::new(5_000.0); // 5 km radius = smaller active set
        assert_eq!(set.active_count(), 0);

        // Refresh at origin — active set should be non-empty.
        set.refresh(glam::Vec2::ZERO, &catalog);
        assert!(set.active_count() > 0, "active set populated at origin");
        assert_eq!(set.exited().len(), 0, "first refresh has no exits");
        assert_eq!(
            set.entered().len(),
            set.active_count(),
            "everything entered on first refresh"
        );

        // Refresh at same position — no deltas.
        set.refresh(glam::Vec2::ZERO, &catalog);
        assert_eq!(set.entered().len(), 0);
        assert_eq!(set.exited().len(), 0);

        // Refresh after moving 20 km — entirely new set expected.
        let initial_active: Vec<_> = set.active().to_vec();
        set.refresh(glam::Vec2::new(20_000.0, 0.0), &catalog);
        let new_active: Vec<_> = set.active().to_vec();
        // Active sets should mostly disagree at 20 km (2× radius) offset.
        let overlap_count = initial_active
            .iter()
            .filter(|id| new_active.contains(id))
            .count();
        assert!(
            overlap_count < initial_active.len() / 2,
            "20 km offset should evict most of the old active set"
        );

        // Active set always sorted ascending.
        let mut sorted = set.active().to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, set.active());
    }
}
