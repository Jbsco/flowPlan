//! Chunked W2 weather streaming (Phase 3.6.3c.6 / 3.2x C1).
//!
//! Universal chunked-Voronoi pattern instance for the W2 weather
//! partition (memory pin `chunked_voronoi_layers_universal`). Streams
//! W2 cell state into a CPU-side cache as cells enter / exit a 50 km
//! radius around the camera. C2 wires the cache into a GPU storage
//! buffer for the cloud raymarcher.
//!
//! # Differences from terrain / water
//!
//! - **Cell scale**: ~25 km Voronoi spacing per W2 design (vs terrain
//!   L2's ~2 km). Active-set count is ~25-50 cells in 50 km radius
//!   (vs ~440 for terrain).
//! - **Cheaper per-cell work**: ~1 µs per cell to sample state via
//!   `cell_state::state_at_time_with_terrain` (vs ~25 ms per terrain
//!   chunk for the mesh build). Worker pool size of 1 is sufficient.
//! - **No mesh artifact**: result is `WeatherCellSnapshot` — a small
//!   POD struct with cell-state + seed position + drift offset.
//! - **Time-varying**: weather cells evolve through life-cycle phases
//!   on minute scales. C1 ships state-at-cell-enter; periodic refresh
//!   is C1+ polish.
//!
//! # Architecture
//!
//! Per the universal pattern (see `crates/app/src/chunked_terrain.rs`
//! for the canonical reference):
//!
//! - **State**: `HashMap<CellId, WeatherChunkState>` tracking cells
//!   currently in the active set.
//! - **Worker pool**: 1 thread (cheap bake, single worker is enough),
//!   shared priority queue (nearest-first by camera distance), result
//!   channel back to main.
//! - **Per-frame `update()`**: refreshes active cell-set →
//!   enqueues entered cells → drains exited cells → drains finished
//!   results from the channel.
//! - **Pop-time staleness check** (3.6.3c.5.1) — workers recompute
//!   distance at pop time; cells that drifted out get dropped.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::needless_pass_by_value)]
#![allow(dead_code)]

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use flowplan_world::terrain::Terrain;
use flowplan_world::weather::cell_state::{self, CellState};
use flowplan_world::weather::voronoi::{self, CellId, GRID_SPACING_M};
use glam::Vec2;

/// Streaming radius for W2 weather cells (m). Larger than terrain's
/// 22 km / water's 15 km — atmospheric horizon is the longest-visible
/// layer, so we want cells well past the camera's nearby coverage.
/// 50 km gives ~5-6 cell-rows in any direction at GRID_SPACING_M = 25 km.
pub const WEATHER_STREAM_RADIUS_M: f32 = 50_000.0;

/// Worker pool size. 1 is sufficient — each cell-state bake is ~1 µs.
/// Universal pattern keeps the worker abstraction even for cheap work
/// so the streaming pipeline is consistent across layers.
pub const MAX_WORKERS: usize = 1;

/// One worker job — re-sample cell state for `cell_id` at `sim_time`.
#[derive(Clone)]
struct WeatherJob {
    cell_id: CellId,
    seed_pos: Vec2,
    sim_time_s: f32,
    priority_dist_m: f32,
}

#[derive(Clone)]
struct JobByPriority(WeatherJob);
impl Eq for JobByPriority {}
impl PartialEq for JobByPriority {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority_dist_m.partial_cmp(&other.0.priority_dist_m)
            == Some(std::cmp::Ordering::Equal)
    }
}
impl Ord for JobByPriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse-ordering for min-heap (nearest first).
        other
            .0
            .priority_dist_m
            .partial_cmp(&self.0.priority_dist_m)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                (self.0.cell_id.ix, self.0.cell_id.iz)
                    .cmp(&(other.0.cell_id.ix, other.0.cell_id.iz))
            })
    }
}
impl PartialOrd for JobByPriority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Per-cell snapshot delivered by the worker. Compact POD form
/// suitable for upload to a GPU storage buffer (C2 will pack into
/// `GpuWeatherCell` for `repr(C)` shader compatibility).
#[derive(Clone, Copy, Debug)]
pub struct WeatherCellSnapshot {
    pub cell_id: CellId,
    pub seed_pos: Vec2,
    pub state: CellState,
}

/// Worker output — finished snapshot for `cell_id`.
pub struct WeatherChunkResult {
    pub snapshot: WeatherCellSnapshot,
}

/// Per-cell state in the manager.
#[derive(Debug)]
pub enum WeatherChunkState {
    /// Worker job in flight.
    Loading,
    /// Snapshot ready; consumer can read it.
    Resident(WeatherCellSnapshot),
}

impl WeatherChunkState {
    pub fn snapshot(&self) -> Option<&WeatherCellSnapshot> {
        match self {
            Self::Resident(s) => Some(s),
            Self::Loading => None,
        }
    }
}

/// Shared worker state.
struct WorkerShared {
    queue: Mutex<BinaryHeap<JobByPriority>>,
    queue_signal: Condvar,
    shutdown: Mutex<bool>,
    result_tx: Sender<WeatherChunkResult>,
    master_seed: u64,
    terrain: Arc<Terrain>,
    /// Current camera position. Written per frame by `update()`; read
    /// at pop time for the staleness check (3.6.3c.5.1).
    camera_pos: std::sync::RwLock<Vec2>,
    /// Streaming radius (m) — copied at construction.
    streaming_radius_m: f32,
}

/// Chunked W2 weather manager.
pub struct ChunkedWeather {
    states: HashMap<CellId, WeatherChunkState>,
    shared: Arc<WorkerShared>,
    result_rx: Receiver<WeatherChunkResult>,
    workers: Vec<thread::JoinHandle<()>>,
    /// Last sim_time at which cell snapshots were taken. Used by the
    /// optional periodic refresh (C1+ polish).
    snapshot_sim_time_s: f32,
}

impl ChunkedWeather {
    /// Spawn the worker pool. `terrain` is used by the
    /// terrain-aware humidity sampler in `state_at_time_with_terrain`
    /// so cell classification reflects orographic lift / rain shadow.
    #[must_use]
    pub fn new(master_seed: u64, terrain: Arc<Terrain>) -> Self {
        let (tx, rx) = channel::<WeatherChunkResult>();
        let shared = Arc::new(WorkerShared {
            queue: Mutex::new(BinaryHeap::new()),
            queue_signal: Condvar::new(),
            shutdown: Mutex::new(false),
            result_tx: tx,
            master_seed,
            terrain,
            camera_pos: std::sync::RwLock::new(Vec2::ZERO),
            streaming_radius_m: WEATHER_STREAM_RADIUS_M,
        });
        let mut workers = Vec::with_capacity(MAX_WORKERS);
        for i in 0..MAX_WORKERS {
            let s = Arc::clone(&shared);
            let h = thread::Builder::new()
                .name(format!("chunked-weather-{i}"))
                .spawn(move || worker_loop(&s))
                .unwrap_or_else(|e| panic!("spawn weather worker {i}: {e}"));
            workers.push(h);
        }
        Self {
            states: HashMap::new(),
            shared,
            result_rx: rx,
            workers,
            snapshot_sim_time_s: 0.0,
        }
    }

    /// Per-frame update. Maintains active set; integrates worker
    /// results. Returns `(entered, exited)` cell-id sets — consumer
    /// uses these to update GPU storage buffer slots in C2.
    pub fn update(&mut self, camera_xz: Vec2, sim_time_s: f32) -> WeatherChunkUpdate {
        // Publish current camera so workers can recompute distance at
        // pop time and skip stale jobs.
        if let Ok(mut pos) = self.shared.camera_pos.write() {
            *pos = camera_xz;
        }

        self.snapshot_sim_time_s = sim_time_s;

        // Compute active cell set. Sample at GRID_SPACING_M / 2 spacing
        // so cells smaller than the average grid (variable-size power-
        // diagram) still get captured.
        let active_set = active_weather_cells(
            self.shared.master_seed,
            camera_xz,
            self.shared.streaming_radius_m,
        );

        // Enqueue jobs for newly-entered cells.
        let mut new_jobs: Vec<WeatherJob> = Vec::new();
        for cell_id in &active_set {
            if !self.states.contains_key(cell_id) {
                let seed_pos = voronoi::seed_position(self.shared.master_seed, *cell_id);
                let dx = seed_pos.x - camera_xz.x;
                let dz = seed_pos.y - camera_xz.y;
                new_jobs.push(WeatherJob {
                    cell_id: *cell_id,
                    seed_pos,
                    sim_time_s,
                    priority_dist_m: dx.hypot(dz),
                });
                self.states.insert(*cell_id, WeatherChunkState::Loading);
            }
        }
        if !new_jobs.is_empty() {
            let mut q = self.shared.queue.lock().expect("weather queue lock");
            for job in new_jobs {
                q.push(JobByPriority(job));
            }
            drop(q);
            self.shared.queue_signal.notify_all();
        }

        // Identify exited cells.
        let mut exited: Vec<CellId> = Vec::new();
        let states_snapshot: Vec<CellId> = self.states.keys().copied().collect();
        for cell_id in states_snapshot {
            if !active_set.contains(&cell_id) {
                self.states.remove(&cell_id);
                exited.push(cell_id);
            }
        }

        // Drain finished results from worker channel.
        let mut entered: Vec<WeatherCellSnapshot> = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
            // Drop result if cell is no longer active (drifted out
            // during the worker's run).
            if !active_set.contains(&result.snapshot.cell_id) {
                self.states.remove(&result.snapshot.cell_id);
                continue;
            }
            entered.push(result.snapshot);
            self.states.insert(
                result.snapshot.cell_id,
                WeatherChunkState::Resident(result.snapshot),
            );
        }

        WeatherChunkUpdate { entered, exited }
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.states
            .values()
            .filter(|s| matches!(s, WeatherChunkState::Loading))
            .count()
    }

    #[must_use]
    pub fn resident_count(&self) -> usize {
        self.states
            .values()
            .filter(|s| matches!(s, WeatherChunkState::Resident(_)))
            .count()
    }

    /// Iterate all currently-resident cell snapshots. C2 uses this
    /// to populate the GPU storage buffer.
    pub fn iter_resident(&self) -> impl Iterator<Item = &WeatherCellSnapshot> {
        self.states.values().filter_map(WeatherChunkState::snapshot)
    }
}

impl Drop for ChunkedWeather {
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

/// Per-update output. Consumer (C2 renderer) uses `entered` to write
/// new GPU buffer slots; `exited` to free the old slot if any.
pub struct WeatherChunkUpdate {
    pub entered: Vec<WeatherCellSnapshot>,
    pub exited: Vec<CellId>,
}

/// Compute the set of W2 cell IDs whose seed positions fall within
/// `radius_m` of `cam`. Uses a finer-than-grid sampling pattern
/// (every GRID_SPACING_M / 2) so cells smaller than the average get
/// captured even with the variable-size power-diagram metric.
#[allow(clippy::cast_possible_truncation)] // small grid count (~32 max)
#[allow(clippy::cast_precision_loss)] // small i32 (≤ ~32) → f32 is exact
fn active_weather_cells(seed: u64, cam: Vec2, radius_m: f32) -> HashSet<CellId> {
    let mut active = HashSet::new();
    // Step at half grid spacing for safety against variable cell sizes.
    let step_m = GRID_SPACING_M * 0.5;
    let radius_steps = (radius_m / step_m).ceil() as i32 + 1;
    let radius_sq = radius_m * radius_m;
    for dx in -radius_steps..=radius_steps {
        for dz in -radius_steps..=radius_steps {
            let x = cam.x + dx as f32 * step_m;
            let z = cam.y + dz as f32 * step_m;
            let dxx = x - cam.x;
            let dzz = z - cam.y;
            if dxx * dxx + dzz * dzz > radius_sq {
                continue;
            }
            let lookup = voronoi::cell_at(seed, x, z);
            active.insert(lookup.cell_id);
        }
    }
    active
}

fn worker_loop(shared: &Arc<WorkerShared>) {
    loop {
        if *shared.shutdown.lock().expect("weather shutdown check") {
            return;
        }
        let job = {
            let mut q = shared.queue.lock().expect("weather queue lock");
            loop {
                if let Some(j) = q.pop() {
                    break Some(j);
                }
                if *shared.shutdown.lock().expect("weather shutdown check") {
                    break None;
                }
                q = shared.queue_signal.wait(q).expect("weather queue cv wait");
            }
        };
        match job {
            Some(JobByPriority(job)) => bake_chunk(shared, job),
            None => return,
        }
    }
}

fn bake_chunk(shared: &Arc<WorkerShared>, job: WeatherJob) {
    // Pop-time staleness check (3.6.3c.5.1): if camera has moved far
    // enough that this cell is now well outside the streaming radius,
    // discard without baking.
    if let Ok(cam) = shared.camera_pos.read() {
        let dx = job.seed_pos.x - cam.x;
        let dz = job.seed_pos.y - cam.y;
        let staleness_radius = shared.streaming_radius_m * 1.5;
        if dx.hypot(dz) > staleness_radius {
            return;
        }
    }

    let state = cell_state::state_at_time_with_terrain(
        shared.master_seed,
        shared.terrain.as_ref(),
        job.cell_id,
        job.sim_time_s,
    );

    let snapshot = WeatherCellSnapshot {
        cell_id: job.cell_id,
        seed_pos: job.seed_pos,
        state,
    };

    drop(shared.result_tx.send(WeatherChunkResult { snapshot }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Helper: spin until the manager has at least one resident cell
    /// or we time out. Worker takes µs to bake but mpsc + thread sched
    /// can introduce small latency.
    fn wait_for_resident(weather: &mut ChunkedWeather, camera: Vec2, time_s: f32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            weather.update(camera, time_s);
            if weather.resident_count() > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn active_cells_within_radius_at_origin() {
        let cells = active_weather_cells(42, Vec2::ZERO, WEATHER_STREAM_RADIUS_M);
        // 50 km radius / 25 km grid = ~5 cells per radius. Total active
        // set should be in the dozens (varies with power-diagram
        // weights).
        assert!(
            cells.len() >= 4 && cells.len() <= 100,
            "active set count {} outside expected range",
            cells.len()
        );
    }

    #[test]
    fn active_cells_decreases_as_radius_shrinks() {
        let big = active_weather_cells(42, Vec2::ZERO, 100_000.0);
        let small = active_weather_cells(42, Vec2::ZERO, 10_000.0);
        assert!(
            big.len() > small.len(),
            "big radius {} should have more cells than small radius {}",
            big.len(),
            small.len()
        );
    }

    #[test]
    fn manager_produces_resident_cells() {
        let terrain = Arc::new(Terrain::from_seed(42));
        let mut weather = ChunkedWeather::new(42, terrain);
        let camera = Vec2::ZERO;
        let got_resident = wait_for_resident(&mut weather, camera, 0.0);
        assert!(got_resident, "no resident cells after 2s — worker stalled?");
        let count = weather.resident_count();
        // At origin with 50 km radius, expect at least a handful of
        // cells.
        assert!(count >= 4, "only {count} resident cells");
    }

    #[test]
    fn manager_evicts_cells_on_camera_move() {
        let terrain = Arc::new(Terrain::from_seed(42));
        let mut weather = ChunkedWeather::new(42, terrain);
        wait_for_resident(&mut weather, Vec2::ZERO, 0.0);
        let initial_cells: HashSet<CellId> = weather.iter_resident().map(|s| s.cell_id).collect();
        assert!(!initial_cells.is_empty());

        // Teleport far across the world. New active set should NOT
        // overlap with original (50 km radius is well clear of the
        // source).
        let far_camera = Vec2::new(1_000_000.0, 1_000_000.0);
        wait_for_resident(&mut weather, far_camera, 0.0);
        let new_cells: HashSet<CellId> = weather.iter_resident().map(|s| s.cell_id).collect();
        assert!(!new_cells.is_empty(), "no cells at remote camera position");
        assert!(
            initial_cells.is_disjoint(&new_cells),
            "new cell set should not overlap original after teleport"
        );
    }

    #[test]
    fn snapshot_cell_state_deterministic() {
        // Two managers with same seed at same camera should produce
        // same cell snapshots. Just spot-check one cell.
        let terrain1 = Arc::new(Terrain::from_seed(42));
        let terrain2 = Arc::new(Terrain::from_seed(42));
        let mut a = ChunkedWeather::new(42, terrain1);
        let mut b = ChunkedWeather::new(42, terrain2);
        wait_for_resident(&mut a, Vec2::ZERO, 100.0);
        wait_for_resident(&mut b, Vec2::ZERO, 100.0);
        let a_states: HashMap<CellId, CellState> =
            a.iter_resident().map(|s| (s.cell_id, s.state)).collect();
        let b_states: HashMap<CellId, CellState> =
            b.iter_resident().map(|s| (s.cell_id, s.state)).collect();
        for (id, sa) in &a_states {
            if let Some(sb) = b_states.get(id) {
                assert_eq!(
                    sa.cell_type, sb.cell_type,
                    "cell {id:?} type differs across managers"
                );
                assert_eq!(
                    sa.density.to_bits(),
                    sb.density.to_bits(),
                    "cell {id:?} density differs"
                );
            }
        }
    }
}
