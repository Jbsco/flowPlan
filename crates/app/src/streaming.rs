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

use flowplan_assets::CpuMesh;
use flowplan_world::terrain::{HeightSampler, Terrain};
use flowplan_world::voronoi_cells::{CellId, SubBiomeCellCatalog};
use glam::Vec2;

/// Streaming terrain default half-extent (m). Patch is 2× this on a
/// side: 10 km half-extent = 20 km × 20 km mesh. Performance slider
/// in Phase 4.8 settings UI exposes this for player tuning.
pub const TERRAIN_HALF_EXTENT_M: f32 = 10_000.0;

/// Streaming terrain default cells per side. 1000 = 20 m per cell at
/// the default 10 km half-extent. ~1 M vertices total.
pub const TERRAIN_CELLS: u32 = 1000;

/// Streaming water default half-extent (m). Matches terrain at 10 km
/// per user direction (rivers / coastlines visible at distance look
/// wrong with smaller water mesh).
pub const WATER_HALF_EXTENT_M: f32 = 10_000.0;

/// Streaming water default cells per side. Coarser than terrain (water
/// mesh is flat in 3.6.3c.5b; FFT ocean displacement adds detail later
/// per `weather_water_fft_second_pass` memory).
pub const WATER_CELLS: u32 = 200;

/// Camera-movement threshold that triggers a mesh rebuild. Measured
/// from the current mesh center.
///
/// Default 2 km = half the radius the camera can travel inside the
/// 10 km mesh before hitting the edge. Smaller threshold → more
/// rebuilds (smoother edge); larger → fewer rebuilds (mesh edge
/// approaches camera). 2 km is the engineering compromise.
pub const REBUILD_THRESHOLD_M: f32 = 2_000.0;

/// Voronoi-cell streaming radius. Cells whose seed position is within
/// this distance of the active camera are in the active set; others
/// are not. Used by Layer 2 content streaming (feature instances,
/// future LOD, hub markers, weather overlays, audio sources).
pub const STREAM_RADIUS_M: f32 = 10_000.0;

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
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
    pub orientation: glam::Quat,
    /// Field of view (used by renderer). Consumed in 3.6.3c.5a.
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
    pub fov_y_radians: f32,
    /// True when the debug-fly camera is active (3.6.3c.5a). Streaming
    /// behavior is identical regardless; the flag is for HUD + input
    /// gating in the debug-camera milestone.
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
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
    /// frame has a mesh to render.
    #[must_use]
    pub fn new(half_extent: f32, resolution: u32, initial_center: Vec2, terrain: &Terrain) -> Self {
        let cpu_mesh = CpuMesh::terrain_patch_at(
            resolution,
            half_extent,
            (initial_center.x, initial_center.y),
            |x, z| terrain.height(x, z),
        );
        Self {
            cpu_mesh,
            center: initial_center,
            half_extent,
            resolution,
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
    /// a new worker with a cloned `Terrain`.
    ///
    /// Main thread work per frame is effectively free — a channel probe
    /// plus a spawn decision. The 1 s-ish mesh-build cost lives on the
    /// worker thread, invisible to the render loop.
    pub fn maybe_rebuild(&mut self, camera_pos_xz: Vec2, terrain: &Terrain) {
        self.just_rebuilt = false;

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
            let terrain_clone = terrain.clone();
            let resolution = self.resolution;
            let half_extent = self.half_extent;
            let target_center = camera_pos_xz;
            let spawn_result = std::thread::Builder::new()
                .name("streaming-terrain-build".into())
                .spawn(move || {
                    let t0 = std::time::Instant::now();
                    let mesh = CpuMesh::terrain_patch_at(
                        resolution,
                        half_extent,
                        (target_center.x, target_center.y),
                        |x, z| terrain_clone.height(x, z),
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
    #[allow(dead_code)] // TODO(3.6.3c.5a): consumed by debug HUD
    pub const fn last_rebuild_cpu_ms(&self) -> f32 {
        self.last_rebuild_cpu_ms
    }

    /// True iff a worker rebuild is currently in flight. Useful for
    /// HUD status ("rebuilding...") + debugging.
    #[must_use]
    #[allow(dead_code)] // TODO(3.6.3c.5a): consumed by debug HUD
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
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
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
    /// Initial bake at the given center.
    #[must_use]
    pub fn new(half_extent: f32, resolution: u32, initial_center: Vec2, color: [f32; 4]) -> Self {
        let cpu_mesh = CpuMesh::water_grid_at(
            resolution,
            half_extent,
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
            self.cpu_mesh = CpuMesh::water_grid_at(
                self.resolution,
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
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
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
    /// O(K) where K is the cell count in radius (~80 at default 10 km
    /// radius / 2 km cell spacing).
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

    /// Currently-active cells (sorted ascending). Consumed by 3.6.3c.5c
    /// feature-instance streaming + 3.6.3c.5a debug HUD (shows active
    /// count). Allow per `CODING_RULES` §61 — consumers queued on Tasks
    /// #223 + future 3.6.3c.5c.
    #[must_use]
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
    pub fn active(&self) -> &[CellId] {
        &self.active
    }

    /// Cells that entered on the most recent refresh. Consumed by
    /// 3.6.3c.5c for per-cell resource allocation (spawn feature
    /// instances, allocate per-cell buffers).
    #[must_use]
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
    pub fn entered(&self) -> &[CellId] {
        &self.entered
    }

    /// Cells that exited on the most recent refresh. Consumed by
    /// 3.6.3c.5c for per-cell resource release (free feature instances,
    /// deallocate per-cell buffers).
    #[must_use]
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
    pub fn exited(&self) -> &[CellId] {
        &self.exited
    }

    /// Number of active cells. Consumed by 3.6.3c.5a debug HUD.
    #[must_use]
    // TODO(3.6.3c.5a/5c): remove this allow when the listed consumer lands.
    #[allow(dead_code)]
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
        // 10 km radius at 2 km cell spacing → ~80 cells active.
        const _: () = assert!(STREAM_RADIUS_M >= 9_000.0);
        const _: () = assert!(STREAM_RADIUS_M <= 12_000.0);
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
        let mut st = StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, glam::Vec2::ZERO, &terrain);
        assert!(st.just_rebuilt(), "initial (sync) build is a rebuild");
        assert_eq!(st.center(), glam::Vec2::ZERO);

        // Movement below threshold should NOT kick off a rebuild.
        st.maybe_rebuild(glam::Vec2::new(REBUILD_THRESHOLD_M * 0.5, 0.0), &terrain);
        assert!(!st.just_rebuilt());
        assert!(!st.rebuild_in_flight());
        assert_eq!(st.center(), glam::Vec2::ZERO, "center unchanged");

        // Movement past threshold kicks off a worker. Poll until
        // finalize swaps the mesh in.
        let new_pos = glam::Vec2::new(REBUILD_THRESHOLD_M * 1.5, 0.0);
        st.maybe_rebuild(new_pos, &terrain);
        assert!(
            st.rebuild_in_flight() || st.just_rebuilt(),
            "expected worker in flight or immediate finalize"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            st.maybe_rebuild(new_pos, &terrain);
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
    fn streaming_terrain_at_rest_is_deterministic() {
        // Same camera position → same mesh (byte-identical vertex data).
        let terrain = Terrain::from_seed(TEST_SEED);
        let center = glam::Vec2::new(12_345.0, -6_789.0);
        let a = StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, center, &terrain);
        let b = StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, center, &terrain);
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
        let st = StreamingTerrain::new(SMALL_EXTENT, SMALL_CELLS, edge, &terrain);
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
