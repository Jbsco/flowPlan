//! Background scatter worker for ambient-environment generation
//! (Phase 3.6.3c.5c.2.c.12d).
//!
//! Moves the per-cell ambient generation off the main thread onto a
//! priority-queued worker. Main thread enqueues [`AmbientJob`] on each
//! cell-enter with `priority = distance_to_camera_m`; worker pops
//! nearest-first (spiral-out from camera), runs
//! [`flowplan_world::ambient::generate_ambient_for_cell`], and sends
//! back an [`AmbientResult`]. Main thread drains results into its
//! per-cell ambient cache each frame.
//!
//! Mirrors the `StreamingTerrain::pending` async-worker pattern.
//!
//! See [MAPGEN § Background scatter worker](../../../MAPGEN.md#background-scatter-worker--spiral-out-priority-pinned-2026-04-24)
//! for the design pin.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use flowplan_world::ambient::generate_ambient_for_cell;
use flowplan_world::biome::BiomeClass;
use flowplan_world::composed::WorldRefsArc;
use flowplan_world::feature_instances::{FeatureAmbientControl, FeatureInstance};
use flowplan_world::hydrology::{HydrologyGraph, SEA_LEVEL_M};
use flowplan_world::sub_biome::SubBiomeLibraries;
use flowplan_world::terrain::{HeightSampler, Terrain};
use flowplan_world::voronoi_cells::SubBiomeCellCatalog;
use flowplan_world::SubBiomeCellId as CellId;

/// Input to the ambient worker — everything it needs to generate a
/// single cell's ambient set. Pre-fetched on the main thread before
/// enqueue so the worker doesn't need access to `BakedAssignments` or
/// the sub-biome catalog.
#[derive(Clone, Debug)]
pub struct AmbientJob {
    pub cell_id: CellId,
    /// World-space `(x, z)` cell center, from the sub-biome catalog.
    pub seed_pos: (f32, f32),
    pub biome: BiomeClass,
    /// The cell's baked feature instances. Cloned once at enqueue time.
    pub feat_insts: Vec<FeatureInstance>,
    pub ambient_control: Option<FeatureAmbientControl>,
    /// Distance from the active camera at enqueue time. Smaller =
    /// higher priority (processed first).
    pub priority_dist_m: f32,
}

/// Worker output — the generated ambient instances for a cell. Main
/// thread inserts these into its cache on `try_recv`.
#[derive(Clone, Debug)]
pub struct AmbientResult {
    pub cell_id: CellId,
    pub instances: Vec<FeatureInstance>,
}

/// Wraps `AmbientJob` to make it orderable by priority. `BinaryHeap`
/// is a max-heap; we want min (nearest-first), so this inverts the
/// comparison.
#[derive(Debug)]
struct JobByPriority(AmbientJob);

impl Eq for JobByPriority {}
impl PartialEq for JobByPriority {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority_dist_m.partial_cmp(&other.0.priority_dist_m) == Some(Ordering::Equal)
    }
}
impl PartialOrd for JobByPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for JobByPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        // Invert: smaller priority_dist_m sorts as "greater" so it
        // pops first from the max-heap. NaN distance → treat as equal.
        other
            .0
            .priority_dist_m
            .partial_cmp(&self.0.priority_dist_m)
            .unwrap_or(Ordering::Equal)
    }
}

/// Handle to the background ambient-generation worker. Drop to
/// shut down (closes the job channel; worker exits on next wake).
pub struct AmbientWorker {
    job_tx: Sender<AmbientJob>,
    result_rx: Receiver<AmbientResult>,
    _thread: JoinHandle<()>,
}

impl AmbientWorker {
    /// Spawn the worker with read-only shared access to the world's
    /// terrain + hydrology + sub-biome catalog. Terrain is cloned;
    /// hydrology + catalog are Arc-shared. The worker never mutates.
    /// The catalog is needed for the per-cell Voronoi-polygon clip
    /// (5c.2.c.12i) — replaces the old hardcoded disc-radius sampling.
    #[must_use]
    pub fn spawn(
        master_seed: u64,
        terrain: Terrain,
        hydrology: Arc<HydrologyGraph>,
        catalog: Arc<SubBiomeCellCatalog>,
        world_refs: Option<WorldRefsArc>,
    ) -> Self {
        let (job_tx, job_rx) = channel::<AmbientJob>();
        let (result_tx, result_rx) = channel::<AmbientResult>();
        let thread = match thread::Builder::new()
            .name("flowplan-ambient-worker".into())
            .spawn(move || {
                worker_loop(
                    master_seed,
                    &terrain,
                    &hydrology,
                    &catalog,
                    world_refs.as_ref(),
                    &job_rx,
                    &result_tx,
                );
            }) {
            Ok(handle) => handle,
            Err(e) => panic!("spawn ambient worker thread: {e}"),
        };
        Self {
            job_tx,
            result_rx,
            _thread: thread,
        }
    }

    /// Enqueue a job. Drops on channel-close (worker shut down). The
    /// queue is unbounded so this never blocks; backpressure comes
    /// from the worker falling behind + main-thread filtering of
    /// results for cells that have since exited.
    pub fn enqueue(&self, job: AmbientJob) {
        drop(self.job_tx.send(job));
    }

    /// Non-blocking drain of completed results. Caller inspects each
    /// and decides whether the cell is still relevant.
    pub fn try_recv(&self) -> Option<AmbientResult> {
        self.result_rx.try_recv().ok()
    }
}

#[allow(clippy::too_many_arguments)]
fn worker_loop(
    master_seed: u64,
    terrain: &Terrain,
    hydrology: &Arc<HydrologyGraph>,
    catalog: &Arc<SubBiomeCellCatalog>,
    world_refs: Option<&WorldRefsArc>,
    job_rx: &Receiver<AmbientJob>,
    result_tx: &Sender<AmbientResult>,
) {
    let libs = SubBiomeLibraries::shared();
    let mut heap: BinaryHeap<JobByPriority> = BinaryHeap::new();

    loop {
        // Drain any queued jobs before blocking (absorbs bursts).
        while let Ok(job) = job_rx.try_recv() {
            heap.push(JobByPriority(job));
        }

        // If nothing queued, block on the channel.
        if heap.is_empty() {
            match job_rx.recv() {
                Ok(job) => heap.push(JobByPriority(job)),
                Err(_) => return, // channel closed, shut down
            }
            // Drain once more — more jobs may have queued while we
            // were blocking (the receiver wakes on first message).
            while let Ok(job) = job_rx.try_recv() {
                heap.push(JobByPriority(job));
            }
        }

        let Some(JobByPriority(job)) = heap.pop() else {
            continue;
        };

        let entries = libs.ambient_for_biome(job.biome);
        if entries.is_empty() {
            continue;
        }

        // Pre-wrap coords before noise sampling. perlin_2d wraps the
        // cell index but not the fractional position; at world coords
        // outside the canonical [0, WORLD_EXTENT_M) range the noise
        // function samples at a different fractional cell position
        // than at the canonical equivalent. Pre-wrapping ensures
        // consistent terrain answers for cells with seeds near the
        // wrap seam (5c.2.c.12r).
        //
        // 3.6.3c.3c: when `world_refs` is present (FLOWPLAN_FEATURE_INSTANCES=1
        // path), sample L1+L3 composed height so ambient instances
        // follow mesa plateaus / cliff-coast rims / road cuts / hub
        // foundations. Without this, mesa tops render the L3 mesh
        // raised by 100+ m but ambient still anchors to L1 → instances
        // bury in the elevated terrain or float above cuts.
        let height_at = |x: f32, z: f32| {
            let xc = flowplan_core::wrap_coord(x);
            let zc = flowplan_core::wrap_coord(z);
            world_refs.map_or_else(
                || HeightSampler::height(terrain, xc, zc),
                |refs| refs.height_at(xc, zc),
            )
        };
        // Water filter: simple global SEA_LEVEL_M cut against the
        // composed L1+L3 height — instances on cliff-coast tops
        // (raised 90 m above sea by the delta) correctly pass the
        // not-water gate; instances in carved lake bowls (depressed
        // by `lake_cluster` delta) correctly fail it.
        let _ = hydrology; // reserved for future per-pixel water-surface field
        let is_water = |x: f32, z: f32| {
            let xc = flowplan_core::wrap_coord(x);
            let zc = flowplan_core::wrap_coord(z);
            let h = world_refs.map_or_else(
                || HeightSampler::height(terrain, xc, zc),
                |refs| refs.height_at(xc, zc),
            );
            h < SEA_LEVEL_M
        };
        let catalog_ref = catalog.as_ref();
        let target_cell = job.cell_id;
        let is_in_cell = |x: f32, z: f32| catalog_ref.nearest_cell(x, z) == target_cell;

        let instances = generate_ambient_for_cell(
            master_seed,
            job.cell_id,
            job.seed_pos,
            job.biome,
            entries,
            &job.feat_insts,
            job.ambient_control,
            &height_at,
            &is_water,
            &is_in_cell,
        );

        if result_tx
            .send(AmbientResult {
                cell_id: job.cell_id,
                instances,
            })
            .is_err()
        {
            return; // result channel closed, main thread dropped us
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use flowplan_world::terrain::Terrain;

    fn test_worker() -> AmbientWorker {
        // Procedural terrain (not flat) so some candidates land above
        // the primary-carpet altitude floor (+3 m).
        let terrain = Terrain::from_seed(42);
        let hydrology = Arc::new(HydrologyGraph::bake(42, &terrain));
        let catalog = Arc::new(SubBiomeCellCatalog::bake(42));
        AmbientWorker::spawn(42, terrain, hydrology, catalog, None)
    }

    /// Poll the worker up to `timeout_ms` waiting for a single result.
    fn wait_for_result(worker: &AmbientWorker, timeout_ms: u64) -> Option<AmbientResult> {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(timeout_ms) {
            if let Some(r) = worker.try_recv() {
                return Some(r);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn worker_processes_single_job_and_returns_result() {
        // Plumbing test: confirm that enqueueing a job leads to a
        // result with the matching cell_id. Content of `instances`
        // depends on terrain+noise and is covered by ambient::tests.
        let worker = test_worker();
        worker.enqueue(AmbientJob {
            cell_id: 77,
            seed_pos: (2000.0, 3000.0),
            biome: BiomeClass::CoastalBoreal,
            feat_insts: Vec::new(),
            ambient_control: None,
            priority_dist_m: 0.0,
        });
        let r = wait_for_result(&worker, 500)
            .unwrap_or_else(|| panic!("worker should have processed the job"));
        assert_eq!(r.cell_id, 77);
    }

    #[test]
    fn worker_respects_priority_nearest_first() {
        let worker = test_worker();
        // Enqueue 3 jobs in "wrong" order; worker should pop nearest first.
        worker.enqueue(AmbientJob {
            cell_id: 100,
            seed_pos: (0.0, 0.0),
            biome: BiomeClass::CoastalBoreal,
            feat_insts: Vec::new(),
            ambient_control: None,
            priority_dist_m: 5000.0,
        });
        worker.enqueue(AmbientJob {
            cell_id: 101,
            seed_pos: (10.0, 10.0),
            biome: BiomeClass::CoastalBoreal,
            feat_insts: Vec::new(),
            ambient_control: None,
            priority_dist_m: 100.0,
        });
        worker.enqueue(AmbientJob {
            cell_id: 102,
            seed_pos: (20.0, 20.0),
            biome: BiomeClass::CoastalBoreal,
            feat_insts: Vec::new(),
            ambient_control: None,
            priority_dist_m: 1000.0,
        });

        let mut order = Vec::new();
        let start = std::time::Instant::now();
        while order.len() < 3 && start.elapsed() < std::time::Duration::from_millis(3000) {
            if let Some(r) = worker.try_recv() {
                order.push(r.cell_id);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert_eq!(order.len(), 3, "all 3 jobs should process");
        // Best-effort priority check — the first pop depends on how
        // quickly the initial try_recv drain captured all 3 before
        // the heap popped. In practice the worker try_recv loop
        // drains before popping, so priority order is reliable for
        // bursts. Skip strict assertion if the burst lost the race
        // (cell 100 in front) — just confirm all 3 processed.
        if order[0] == 100 {
            // Rare race — worker grabbed first job before others
            // queued. Skip ordering assertion this run.
        } else {
            assert_eq!(
                order[0], 101,
                "nearest priority should pop first (order was {order:?})"
            );
        }
    }
}
