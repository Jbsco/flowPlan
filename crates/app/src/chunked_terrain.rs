//! Chunked terrain streaming (Phase 3.6.3c.4).
//!
//! Replaces the monolithic `StreamingTerrain` (one big disc-mesh
//! rebuilt as a unit) with **per-Voronoi-cell mesh chunks**. As cells
//! enter or exit the streaming radius, the manager spawns async build
//! jobs / drops their GPU meshes. Per-frame cost on the main thread is
//! a queue probe + at most a handful of GPU uploads.
//!
//! # Architecture
//!
//! - **State**: `HashMap<CellId, ChunkState>`. States: `Loading`
//!   (worker job in flight), `Loaded(GpuMesh)` (ready to render),
//!   `Pending` (in queue but not started — covers the case where the
//!   worker pool is saturated).
//! - **Workers**: a fixed-size pool of N threads (`max_workers`).
//!   Each pulls one job from a shared priority queue (sorted by
//!   distance from camera, nearest-first), builds the chunk, sends
//!   the result back via channel.
//! - **Per-frame `update()`**: refreshes `StreamingCellSet` →
//!   enqueues entered cells → drains exited cells → drains finished
//!   results from the channel → uploads new GPU meshes.
//! - **Render**: `iter_renderable()` returns `(CellId, &GpuMesh)`
//!   pairs for the renderer to draw.
//!
//! # Boundary continuity
//!
//! Each chunk samples a **world-aligned grid** (origin at world (0,0)).
//! Adjacent cells' chunks share the exact same vertex positions at
//! their boundary, so heights match (pure function of `(x, z)`) → no
//! z-fight, no gaps, no T-junctions. Out-of-cell vertices are dropped
//! from the VBO via compression in `CpuMesh::terrain_voronoi_chunk`.
//!
//! # Universal pattern
//!
//! This module is the template — water (`ChunkedWater`), volumetric
//! weather (`ChunkedWeather`), future cloud / aurora layers should
//! follow the same shape: per-cell async build, priority queue,
//! per-frame integrate-with-cell-set, renderer-iterates-active-set.

// Worker module — lots of mutex/condvar plumbing where `expect` on
// poison errors is the standard pattern (we'd panic anyway). Inline
// allows scoped to this file per CODING_RULES.md §61.
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
use flowplan_world::composed::WorldRefsArc;
use flowplan_world::voronoi_cells::{CellId, SubBiomeCellCatalog};
use glam::Vec2;

/// World-grid spacing (m) for chunk vertex sampling. Drives mesh
/// resolution + memory + build cost. 30 m = matches the prior
/// monolithic mesh's per-vertex spacing; reasonable tradeoff between
/// terrain detail and bake cost.
pub const CHUNK_GRID_M: f32 = 30.0;

/// Maximum cell-radius used to size each chunk's bounding box. Cells
/// can extend up to this distance from their seed; the chunk builder
/// covers `seed ± MAX_CELL_RADIUS_M` and keeps only in-cell vertices.
/// Bridson observed max neighbor-spacing 3500 m → polygon radius
/// 1750 m; 4000 m gives margin for variable-Voronoi outliers.
pub const MAX_CELL_RADIUS_M: f32 = 4_000.0;

/// Number of worker threads building chunks in parallel. Each chunk
/// takes ~30-200 ms; 4 workers can drain a 5-cell burst in 50-200 ms
/// rather than 150-1000 ms serial.
pub const MAX_WORKERS: usize = 4;

/// One worker job — build a chunk for `cell_id`. Held in the priority
/// queue (sorted by `priority_dist_m`, nearest-first).
#[derive(Clone)]
struct ChunkJob {
    cell_id: CellId,
    seed_pos: (f32, f32),
    /// Distance from active camera at enqueue time. Smaller = higher
    /// priority. Recomputed when the camera moves significantly (a
    /// requeue path is a future polish item; for v1 we accept that
    /// jobs may be slightly mis-prioritized after long flights).
    priority_dist_m: f32,
}

#[derive(Clone)]
struct JobByPriority(ChunkJob);
impl Eq for JobByPriority {}
impl PartialEq for JobByPriority {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority_dist_m.partial_cmp(&other.0.priority_dist_m)
            == Some(std::cmp::Ordering::Equal)
    }
}
impl Ord for JobByPriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse so smaller dist pops first (BinaryHeap is max-heap).
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

/// Worker output — the freshly-built CPU mesh for `cell_id`. Main
/// thread uploads it to a `GpuMesh` on the next frame.
pub struct ChunkResult {
    pub cell_id: CellId,
    pub mesh: CpuMesh,
}

/// Per-cell chunk state on the main thread.
#[derive(Debug)]
pub enum ChunkState {
    /// Job is in queue or in-flight; no GPU mesh yet.
    Loading,
    /// CPU mesh arrived but renderer-side GPU upload is pending.
    /// Drained next frame.
    Pending(CpuMesh),
}

/// Shared job queue + result channel. The worker pool pulls jobs
/// from `queue`; main thread pushes results into `result_tx`.
struct WorkerShared {
    /// Job queue + condvar for wake-on-enqueue. Workers block on the
    /// condvar when the queue is empty; main thread calls
    /// `notify_all()` after pushing.
    queue: Mutex<BinaryHeap<JobByPriority>>,
    queue_signal: Condvar,
    /// `true` once the manager is dropped — workers exit on next wake.
    shutdown: Mutex<bool>,
    /// Result channel — workers send finished `ChunkResult`s.
    result_tx: Sender<ChunkResult>,
    /// World refs for height sampling (composed L1+L3) + cell catalog.
    world_refs: WorldRefsArc,
}

/// Chunked terrain manager.
pub struct ChunkedTerrain {
    /// Per-cell state. Loaded chunks (with GpuMesh) live in the
    /// renderer; `ChunkedTerrain` only tracks Loading / Pending. Once
    /// uploaded, the entry is removed.
    states: HashMap<CellId, ChunkState>,
    /// Tracks which cells have a GpuMesh resident in the renderer.
    /// We don't hold the mesh ourselves — renderer owns it; we just
    /// track which cells are loaded so we know what to drop on exit.
    resident: std::collections::HashSet<CellId>,
    /// Worker pool shared state.
    shared: Arc<WorkerShared>,
    /// Result channel receiver (drained per-frame in `update`).
    result_rx: Receiver<ChunkResult>,
    /// Worker thread handles (joined on drop).
    workers: Vec<thread::JoinHandle<()>>,
}

impl ChunkedTerrain {
    /// Spawn the worker pool with shared world refs. Workers stay
    /// idle until `enqueue_cell` is called.
    #[must_use]
    pub fn new(world_refs: WorldRefsArc) -> Self {
        let (tx, rx) = channel::<ChunkResult>();
        let shared = Arc::new(WorkerShared {
            queue: Mutex::new(BinaryHeap::new()),
            queue_signal: Condvar::new(),
            shutdown: Mutex::new(false),
            result_tx: tx,
            world_refs,
        });
        let mut workers = Vec::with_capacity(MAX_WORKERS);
        for i in 0..MAX_WORKERS {
            let s = Arc::clone(&shared);
            let h = thread::Builder::new()
                .name(format!("chunked-terrain-{i}"))
                .spawn(move || worker_loop(&s))
                .unwrap_or_else(|e| panic!("spawn chunk worker {i}: {e}"));
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

    /// Per-frame entry. Reconciles the active cell set with the chunk
    /// state map: enqueue jobs for newly-entered cells, mark exited
    /// cells for eviction, drain finished results.
    ///
    /// Returns:
    /// - `entered_cell_ids` — cells whose mesh is now CPU-ready (drain
    ///   into renderer GPU upload)
    /// - `exited_cell_ids` — cells whose GPU mesh should be freed
    pub fn update(
        &mut self,
        active_cells: &[CellId],
        catalog: &SubBiomeCellCatalog,
        camera_xz: Vec2,
    ) -> ChunkUpdate {
        // 1. Detect newly-entered cells (in active_cells but not in
        //    states + not in resident). Enqueue their build jobs.
        let active_set: std::collections::HashSet<CellId> = active_cells.iter().copied().collect();
        let mut new_jobs: Vec<ChunkJob> = Vec::new();
        for &cell_id in &active_set {
            if !self.states.contains_key(&cell_id) && !self.resident.contains(&cell_id) {
                let raw = catalog.seeds()[cell_id as usize].position;
                // Wrap seed-pos to be near the camera so the chunk
                // builds at a frame coherent with where the cell's
                // content (instances, ambient) lands. Same fix as the
                // ambient worker (5c.2.c.12m).
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
                new_jobs.push(ChunkJob {
                    cell_id,
                    seed_pos: (sx, sz),
                    priority_dist_m: dx.hypot(dz),
                });
                self.states.insert(cell_id, ChunkState::Loading);
            }
        }
        if !new_jobs.is_empty() {
            let mut q = self.shared.queue.lock().expect("chunk queue lock");
            for job in new_jobs {
                q.push(JobByPriority(job));
            }
            drop(q);
            self.shared.queue_signal.notify_all();
        }

        // 2. Detect exited cells. Drop their state; mark for renderer
        //    eviction.
        let mut exited: Vec<CellId> = Vec::new();
        // Resident cells leaving the active set → evict.
        let resident_snapshot: Vec<CellId> = self.resident.iter().copied().collect();
        for cell_id in resident_snapshot {
            if !active_set.contains(&cell_id) {
                self.resident.remove(&cell_id);
                exited.push(cell_id);
            }
        }
        // States (Loading / Pending) for cells that left → drop without
        // marking for eviction (renderer never had a mesh).
        let states_snapshot: Vec<CellId> = self.states.keys().copied().collect();
        for cell_id in states_snapshot {
            if !active_set.contains(&cell_id) {
                self.states.remove(&cell_id);
            }
        }

        // 3. Drain finished worker results into Pending state. The
        //    actual GPU upload happens in the caller (which has
        //    `&mut Renderer`).
        let mut entered: Vec<(CellId, CpuMesh)> = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
            // The cell may have exited while the worker built it —
            // discard if so.
            if !active_set.contains(&result.cell_id) {
                self.states.remove(&result.cell_id);
                continue;
            }
            self.states
                .insert(result.cell_id, ChunkState::Pending(result.mesh.clone()));
            entered.push((result.cell_id, result.mesh));
            self.resident.insert(result.cell_id);
            self.states.remove(&entered.last().expect("just pushed").0);
        }

        ChunkUpdate { entered, exited }
    }

    /// True if all active cells have a resident GPU mesh (no
    /// `Loading` / `Pending` states left). Useful for HUD / load
    /// indicator.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.states.is_empty()
    }

    /// Number of cells currently in-flight or pending GPU upload.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.states.len()
    }

    /// Number of resident GPU meshes (active set, fully loaded).
    #[must_use]
    pub fn resident_count(&self) -> usize {
        self.resident.len()
    }
}

impl Drop for ChunkedTerrain {
    fn drop(&mut self) {
        // Signal shutdown + wake all workers so they exit on next
        // iteration. We do NOT join — workers may be mid-`build_chunk`
        // (~50-200 ms of mesh sampling) and joining would block ESC /
        // app close until the build finishes. Worker threads hold
        // `Arc<WorldRefsArc>`; OS reclaims them when the process
        // exits. For mid-process drops (replacing a `ChunkedTerrain`
        // without process exit), this would leak threads — not a
        // current concern, queued for follow-up if it lands as a real
        // pattern.
        if let Ok(mut shutdown) = self.shared.shutdown.lock() {
            *shutdown = true;
        }
        self.shared.queue_signal.notify_all();
        // Drain handles without joining — they exit on their own.
        self.workers.clear();
    }
}

/// Per-frame output of `ChunkedTerrain::update`.
pub struct ChunkUpdate {
    /// CPU meshes for cells whose worker just finished. Caller uploads
    /// to GPU + tells renderer to track them.
    pub entered: Vec<(CellId, CpuMesh)>,
    /// Cells whose GPU mesh should be evicted from the renderer.
    pub exited: Vec<CellId>,
}

fn worker_loop(shared: &Arc<WorkerShared>) {
    loop {
        if *shared.shutdown.lock().expect("worker shutdown check") {
            return;
        }
        // Pop a job, or wait on the queue mutex until something arrives.
        let job = {
            let mut q = shared.queue.lock().expect("worker queue lock");
            loop {
                if let Some(j) = q.pop() {
                    break Some(j);
                }
                if *shared.shutdown.lock().expect("worker shutdown check") {
                    break None;
                }
                q = shared.queue_signal.wait(q).expect("worker queue cv wait");
            }
        };
        match job {
            Some(JobByPriority(job)) => build_chunk(shared, job),
            None => return,
        }
    }
}

fn build_chunk(shared: &Arc<WorkerShared>, job: ChunkJob) {
    let cell_id = job.cell_id;
    let (sx, sz) = job.seed_pos;
    let bbox_min = (sx - MAX_CELL_RADIUS_M, sz - MAX_CELL_RADIUS_M);
    let bbox_max = (sx + MAX_CELL_RADIUS_M, sz + MAX_CELL_RADIUS_M);

    let world_refs = shared.world_refs.clone();
    let catalog = Arc::clone(&world_refs.sub_biome);
    let in_cell = |x: f32, z: f32| -> bool {
        catalog.nearest_cell(flowplan_core::wrap_coord(x), flowplan_core::wrap_coord(z)) == cell_id
    };
    let sample_height = |x: f32, z: f32| -> f32 {
        world_refs.height_at(flowplan_core::wrap_coord(x), flowplan_core::wrap_coord(z))
    };

    let mesh = CpuMesh::terrain_voronoi_chunk(
        bbox_min,
        bbox_max,
        CHUNK_GRID_M,
        in_cell,
        sample_height,
        &format!("terrain-chunk-{cell_id}"),
    );

    // Best-effort send; if main thread dropped the receiver, we exit.
    drop(shared.result_tx.send(ChunkResult { cell_id, mesh }));
}
