//! Chunked water streaming (Phase 3.6.3c.5).
//!
//! Mirror of [`crate::chunked_terrain`] but for the water surface.
//! Per-Voronoi-cell flat-quad meshes; same async worker pool +
//! priority queue + per-frame `update()` integration with
//! `StreamingCellSet`. The water mesh follows the established
//! universal chunked-Voronoi pattern (memory pin
//! `chunked_voronoi_layers_universal`).
//!
//! # Differences from terrain
//!
//! - **Smaller streaming radius** ([`WATER_STREAM_RADIUS_M`] = 15 km
//!   vs terrain's 22 km). Water visibility is horizon-limited; no
//!   point streaming water for a cell that's behind a mountain or
//!   beyond the visible horizon.
//! - **Cheaper per-chunk build**: ~1 ms (no fbm height eval, no
//!   gradient sample, no L3 delta dispatch — water is flat Y=0).
//! - **Smaller worker pool** (2 vs 4): build is so cheap that
//!   parallelization is barely needed.
//! - **No `WorldRefsArc` dependency**: only `SubBiomeCellCatalog`
//!   for the `nearest_cell` test. Water doesn't sample terrain.
//!
//! # FFT integration (Phase 3.5b future)
//!
//! Each chunk becomes the unit of FFT displacement. Per-cell uniform
//! buffers can carry chop / wave-direction / amplitude (eventually
//! sourced from the `weather_voronoi` overlay — chunked weather, also
//! per-cell). Vertex shader displaces vertex Y per the FFT texture.
//! Boundary continuity preserved by world-aligned grid: adjacent
//! cells' shared vertex positions get identical FFT samples.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::needless_pass_by_value)]
#![allow(dead_code)]

use std::collections::{BinaryHeap, HashMap};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use flowplan_assets::CpuMesh;
use flowplan_world::voronoi_cells::{CellId, SubBiomeCellCatalog};
use glam::Vec2;

/// World-grid spacing (m) for water vertex sampling. 60 m gives ~half
/// the terrain density since water doesn't need fine detail until
/// FFT lands. Doubles the per-chunk vertex count budget at FFT time.
pub const WATER_GRID_M: f32 = 60.0;

/// Maximum cell radius for water-chunk bbox sizing. Same as terrain.
pub const MAX_WATER_CELL_RADIUS_M: f32 = 4_000.0;

/// Water streaming radius. Smaller than terrain's 22 km — water
/// visibility is horizon-limited; cells farther away are behind
/// terrain or below the visible horizon and don't need streaming.
pub const WATER_STREAM_RADIUS_M: f32 = 15_000.0;

/// Worker pool size for water chunk builds. Smaller than terrain's 4
/// because each water build is ~1 ms — parallel saturation is
/// pointless.
pub const MAX_WORKERS: usize = 2;

/// One worker job — build a water chunk for `cell_id`.
#[derive(Clone)]
struct WaterJob {
    cell_id: CellId,
    seed_pos: (f32, f32),
    priority_dist_m: f32,
}

#[derive(Clone)]
struct JobByPriority(WaterJob);
impl Eq for JobByPriority {}
impl PartialEq for JobByPriority {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority_dist_m.partial_cmp(&other.0.priority_dist_m)
            == Some(std::cmp::Ordering::Equal)
    }
}
impl Ord for JobByPriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .0
            .priority_dist_m
            .partial_cmp(&self.0.priority_dist_m)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(self.0.cell_id.cmp(&other.0.cell_id))
    }
}
impl PartialOrd for JobByPriority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Worker output — finished water mesh for `cell_id`.
pub struct WaterChunkResult {
    pub cell_id: CellId,
    pub mesh: CpuMesh,
}

/// Per-cell chunk state.
#[derive(Debug)]
pub enum WaterChunkState {
    Loading,
    Pending(CpuMesh),
}

/// Shared worker state.
struct WorkerShared {
    queue: Mutex<BinaryHeap<JobByPriority>>,
    queue_signal: Condvar,
    shutdown: Mutex<bool>,
    result_tx: Sender<WaterChunkResult>,
    catalog: Arc<SubBiomeCellCatalog>,
    color: [f32; 4],
    /// Current camera position. Written per frame by `update()`; read
    /// at pop time for the staleness check (3.6.3c.5.1).
    camera_pos: std::sync::RwLock<Vec2>,
    /// Water streaming radius (m) — copied at construction.
    streaming_radius_m: f32,
}

/// Chunked water manager.
pub struct ChunkedWater {
    states: HashMap<CellId, WaterChunkState>,
    resident: std::collections::HashSet<CellId>,
    shared: Arc<WorkerShared>,
    result_rx: Receiver<WaterChunkResult>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl ChunkedWater {
    /// Spawn the worker pool. `color` is the uniform water color
    /// applied to every chunk's vertex — same value the legacy
    /// `StreamingWater` used.
    #[must_use]
    pub fn new(catalog: Arc<SubBiomeCellCatalog>, color: [f32; 4]) -> Self {
        let (tx, rx) = channel::<WaterChunkResult>();
        let shared = Arc::new(WorkerShared {
            queue: Mutex::new(BinaryHeap::new()),
            queue_signal: Condvar::new(),
            shutdown: Mutex::new(false),
            result_tx: tx,
            catalog,
            color,
            camera_pos: std::sync::RwLock::new(Vec2::ZERO),
            streaming_radius_m: WATER_STREAM_RADIUS_M,
        });
        let mut workers = Vec::with_capacity(MAX_WORKERS);
        for i in 0..MAX_WORKERS {
            let s = Arc::clone(&shared);
            let h = thread::Builder::new()
                .name(format!("chunked-water-{i}"))
                .spawn(move || worker_loop(&s))
                .unwrap_or_else(|e| panic!("spawn water worker {i}: {e}"));
            workers.push(h);
        }
        Self {
            states: HashMap::new(),
            resident: std::collections::HashSet::new(),
            shared,
            result_rx: rx,
            workers,
        }
    }

    /// Per-frame update. Same shape as `ChunkedTerrain::update`.
    pub fn update(
        &mut self,
        active_cells: &[CellId],
        catalog: &SubBiomeCellCatalog,
        camera_xz: Vec2,
    ) -> WaterChunkUpdate {
        // Publish current camera so workers can recompute distance at
        // pop time and skip stale jobs (3.6.3c.5.1).
        if let Ok(mut pos) = self.shared.camera_pos.write() {
            *pos = camera_xz;
        }

        let active_set: std::collections::HashSet<CellId> = active_cells.iter().copied().collect();
        let mut new_jobs: Vec<WaterJob> = Vec::new();
        for &cell_id in &active_set {
            if !self.states.contains_key(&cell_id) && !self.resident.contains(&cell_id) {
                let raw = catalog.seeds()[cell_id as usize].position;
                let half = flowplan_core::WORLD_EXTENT_M * 0.5;
                let sx = if raw.0 - camera_xz.x > half {
                    raw.0 - flowplan_core::WORLD_EXTENT_M
                } else if raw.0 - camera_xz.x < -half {
                    raw.0 + flowplan_core::WORLD_EXTENT_M
                } else {
                    raw.0
                };
                let sz = if raw.1 - camera_xz.y > half {
                    raw.1 - flowplan_core::WORLD_EXTENT_M
                } else if raw.1 - camera_xz.y < -half {
                    raw.1 + flowplan_core::WORLD_EXTENT_M
                } else {
                    raw.1
                };
                let dx = sx - camera_xz.x;
                let dz = sz - camera_xz.y;
                new_jobs.push(WaterJob {
                    cell_id,
                    seed_pos: (sx, sz),
                    priority_dist_m: dx.hypot(dz),
                });
                self.states.insert(cell_id, WaterChunkState::Loading);
            }
        }
        if !new_jobs.is_empty() {
            let mut q = self.shared.queue.lock().expect("water queue lock");
            for job in new_jobs {
                q.push(JobByPriority(job));
            }
            drop(q);
            self.shared.queue_signal.notify_all();
        }

        let mut exited: Vec<CellId> = Vec::new();
        let resident_snapshot: Vec<CellId> = self.resident.iter().copied().collect();
        for cell_id in resident_snapshot {
            if !active_set.contains(&cell_id) {
                self.resident.remove(&cell_id);
                exited.push(cell_id);
            }
        }
        let states_snapshot: Vec<CellId> = self.states.keys().copied().collect();
        for cell_id in states_snapshot {
            if !active_set.contains(&cell_id) {
                self.states.remove(&cell_id);
            }
        }

        let mut entered: Vec<(CellId, CpuMesh)> = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
            if !active_set.contains(&result.cell_id) {
                self.states.remove(&result.cell_id);
                continue;
            }
            entered.push((result.cell_id, result.mesh));
            self.resident.insert(result.cell_id);
            self.states.remove(&result.cell_id);
        }

        WaterChunkUpdate { entered, exited }
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.states.len()
    }

    #[must_use]
    pub fn resident_count(&self) -> usize {
        self.resident.len()
    }
}

impl Drop for ChunkedWater {
    fn drop(&mut self) {
        if let Ok(mut shutdown) = self.shared.shutdown.lock() {
            *shutdown = true;
        }
        self.shared.queue_signal.notify_all();
        // Detach workers — same discipline as ChunkedTerrain (avoid
        // ESC-hang on mid-build).
        self.workers.clear();
    }
}

pub struct WaterChunkUpdate {
    pub entered: Vec<(CellId, CpuMesh)>,
    pub exited: Vec<CellId>,
}

fn worker_loop(shared: &Arc<WorkerShared>) {
    loop {
        if *shared.shutdown.lock().expect("water shutdown check") {
            return;
        }
        let job = {
            let mut q = shared.queue.lock().expect("water queue lock");
            loop {
                if let Some(j) = q.pop() {
                    break Some(j);
                }
                if *shared.shutdown.lock().expect("water shutdown check") {
                    break None;
                }
                q = shared.queue_signal.wait(q).expect("water queue cv wait");
            }
        };
        match job {
            Some(JobByPriority(job)) => build_chunk(shared, job),
            None => return,
        }
    }
}

fn build_chunk(shared: &Arc<WorkerShared>, job: WaterJob) {
    let cell_id = job.cell_id;
    let (sx, sz) = job.seed_pos;

    // Pop-time staleness check (3.6.3c.5.1): if camera has moved far
    // enough that this cell is now well outside the streaming radius,
    // discard without building. Same hysteresis (1.5×) as terrain.
    // Water builds are cheaper (~1 ms vs ~25 ms terrain) so the win
    // per skip is smaller, but cost of the check is also tiny — keeps
    // the universal-pattern symmetric.
    if let Ok(cam) = shared.camera_pos.read() {
        let dx = sx - cam.x;
        let dz = sz - cam.y;
        let staleness_radius = shared.streaming_radius_m * 1.5;
        if dx.hypot(dz) > staleness_radius {
            return;
        }
    }

    let bbox_min = (sx - MAX_WATER_CELL_RADIUS_M, sz - MAX_WATER_CELL_RADIUS_M);
    let bbox_max = (sx + MAX_WATER_CELL_RADIUS_M, sz + MAX_WATER_CELL_RADIUS_M);

    let catalog = Arc::clone(&shared.catalog);
    let in_cell = |x: f32, z: f32| -> bool {
        catalog.nearest_cell(flowplan_core::wrap_coord(x), flowplan_core::wrap_coord(z)) == cell_id
    };

    let mesh = CpuMesh::water_voronoi_chunk(
        bbox_min,
        bbox_max,
        WATER_GRID_M,
        in_cell,
        shared.color,
        &format!("water-chunk-{cell_id}"),
    );

    drop(shared.result_tx.send(WaterChunkResult { cell_id, mesh }));
}
