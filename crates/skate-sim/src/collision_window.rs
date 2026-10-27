//! Streamed collision for the big districts (skate_vita M10 stage 1, PLAN.md §15).
//!
//! A district's collision is too big to hold in the simulation's world (about 150 bytes
//! per triangle). Here the world holds only the clusters near the skater: a `Window` is a
//! `BoardWorld` built from the RWCM clusters whose bounds meet a box around the skater,
//! **in archive order**, so the per-mesh and per-triangle indices (which sort their
//! results back into source order, `query_index.rs`/`triangle_index.rs`) return the same
//! candidates in the same relative order as the full world. A query sees the same
//! world as long as every triangle it can reach is inside the window; the window is
//! rebuilt around the skater well before that stops being true.
//!
//! The cluster decoding is a copy of `skate_world::retail_collision_world` (lines
//! 137–212), per cluster so that only the clusters a window needs are decoded.
use skate_core::{
    math::Vector3,
    physics::{
        board_world::{
            BoardWorld, WorldTriangle,
            query_metadata::{Bounds, QueryMesh, QueryMetadata, QueryPool},
        },
        collision::TriangleFeature,
        contact::RetailContactMaterial,
        drive_frames::RetailAffineTransform,
    },
};
/// Emptied buffers of worlds this module built, for the next ones (device only: the background
/// builder's free thread, a session's end and the respawn checks fill it). A fragmented Vita heap
/// (with the retail audio) may have no fresh 10–20 MB block for a window.
static RECYCLED: std::sync::Mutex<Vec<skate_core::physics::board_world::WorldBuffers>> = std::sync::Mutex::new(Vec::new());
/// Sets kept: the current window, the one being built, a respawn check's, and one more for a
/// session that is ending while the next loads.
const KEEP_SETS: usize = 4;

/// Gives a world's buffers back for the next window (device only; a no-op without a background
/// builder). The largest sets are kept.
pub(crate) fn recycle(world: BoardWorld) {
    if !BACKGROUND.load(Ordering::Relaxed) {
        return;
    }
    let buffers = world.into_buffers();
    if buffers.triangles.capacity() == 0 {
        return;
    }
    if let Ok(mut pool) = RECYCLED.lock() {
        pool.push(buffers);
        pool.sort_by_key(|b| std::cmp::Reverse(b.triangles.capacity()));
        pool.truncate(KEEP_SETS);
    }
}

/// Reserves `sets` buffer sets for `triangles` triangles each, at boot, before the heap
/// fragments (the Vita: a district switch with the retail audio found no 10.8 MB block). They
/// are never freed: every streamed world takes one and gives it back.
pub fn reserve_buffers(sets: usize, triangles: usize) {
    if let Ok(mut pool) = RECYCLED.lock() {
        for _ in 0..sets {
            pool.push(skate_core::physics::board_world::WorldBuffers {
                triangles: Vec::with_capacity(triangles),
                triangle_bounds: Vec::with_capacity(triangles),
                packed_surfaces: Vec::with_capacity(triangles),
            });
        }
    }
}

/// The smallest kept set that holds `triangles` (none: an empty one).
fn take_buffers(triangles: usize) -> skate_core::physics::board_world::WorldBuffers {
    let Ok(mut pool) = RECYCLED.lock() else { return Default::default() };
    let pick = pool.iter().rposition(|b| b.triangles.capacity() >= triangles);
    pick.map(|i| pool.remove(i)).unwrap_or_default()
}
use std::sync::atomic::{AtomicU32, Ordering};

use crate::skate_world::decode_native_edges;
use skate_data::retail_collision::{
    ArchiveSource, ClusterInfo, ClusterLoc, ClusterParams, ClusterSource, decode_cluster_bytes, index_archive,
};
use std::collections::HashMap;
use std::sync::Arc;

/// The window's half side and the distance from its centre at which it is rebuilt, in
/// metres, as f32 bits (0 = windows off). Set once at start-up (`configure`).
static BACKGROUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Run first on the builder thread (the device pins it to a spare core).
static INIT: std::sync::Mutex<Option<fn()>> = std::sync::Mutex::new(None);
static HALF: AtomicU32 = AtomicU32::new(0);
static TRIGGER: AtomicU32 = AtomicU32::new(0);

/// Turns streamed collision on for sessions loaded afterwards: the world holds the
/// clusters within `half` metres (a box) of the skater and is rebuilt once the skater
/// is `trigger` metres from the centre it was built at. `half` 0 turns it off.
pub fn configure(half: f32, trigger: f32) {
    HALF.store(half.to_bits(), Ordering::Relaxed);
    TRIGGER.store(trigger.to_bits(), Ordering::Relaxed);
}

/// Builds windows on a thread of their own (the device), `init` running first on it. Without
/// this the window is built inside the tick (the desktop oracle, deterministic).
pub fn configure_background(init: Option<fn()>) {
    BACKGROUND.store(true, Ordering::Relaxed);
    *INIT.lock().unwrap() = init;
}

pub(crate) fn configured() -> Option<(f32, f32)> {
    let half = f32::from_bits(HALF.load(Ordering::Relaxed));
    (half > 0.0).then(|| (half, f32::from_bits(TRIGGER.load(Ordering::Relaxed))))
}

struct ClusterEntry {
    bounds: Bounds,
    triangles: u32,
    params: ClusterParams,
}

/// A district's collision as clusters that are decoded when a window needs them: only each
/// cluster's box, size and decoding parameters stay resident; the bytes come from a
/// `ClusterSource` (the archive in memory, or a district pack read by offset) and the decoded
/// triangles live only in the windows.
/// One query mesh of a cluster, decoded.
struct Run {
    triangles: Vec<WorldTriangle>,
    surfaces: Vec<u16>,
    bounds: Bounds,
    group: i32,
    /// skate_vita (L1): each triangle's broadphase bounds and the mesh's hierarchy over them,
    /// computed once when the cluster is decoded (rebuilding them for every window was most of
    /// a window build on the Vita: 0.4-1.4 s of 50,000 triangles).
    triangle_bounds: Vec<Bounds>,
    tree: skate_core::physics::board_world::MeshTree,
}

pub(crate) struct ClusterSet {
    source: Arc<dyn ClusterSource>,
    /// The clusters of the last window built, decoded (see `world`).
    cache: std::sync::Mutex<HashMap<usize, Arc<Vec<Run>>>>,
    clusters: Vec<ClusterEntry>,
    material: RetailContactMaterial,
    /// The broadphase paddings of the whole world, carried by every window.
    limits: (f32, f32),
}

/// The whole world's broadphase paddings (`BoardWorld::new`) from its widest triangle: a world
/// of just that triangle has them (every triangle's fatness is 0). The district pack stores
/// them so that opening it decodes nothing.
pub fn broadphase_limits(widest: [[f32; 3]; 3]) -> Result<(f32, f32), String> {
    let material = RetailContactMaterial { static_friction: 0., dynamic_friction: 0., restitution: 0. };
    Ok(BoardWorld::new(vec![WorldTriangle::from_vertices(
        widest.map(|p| Vector3::new(p[0], p[1], p[2])),
        material,
        0,
        0x1e1,
        [-1.; 3],
        0.,
    )
    .ok_or("Invalid RWCM collision triangle")?])
    .broadphase_limits())
}

/// A district's collision for the next session to load: the clusters, where their bytes are
/// and the broadphase limits (`install_district`).
pub struct DistrictCollision {
    pub clusters: Vec<ClusterInfo>,
    pub source: Arc<dyn ClusterSource>,
    pub limits: (f32, f32),
}
static INSTALLED: std::sync::Mutex<Option<DistrictCollision>> = std::sync::Mutex::new(None);

/// Hands a district pack's collision to the next session that loads a map without an RWCM
/// archive (its collision is streamed from the pack).
pub fn install_district(district: DistrictCollision) {
    *INSTALLED.lock().unwrap() = Some(district);
}

/// Whether a district pack's collision is waiting for the next session.
pub(crate) fn district_installed() -> bool {
    INSTALLED.lock().unwrap().is_some()
}

impl ClusterSet {
    pub(crate) fn from_archive(archive: Vec<u8>, material: RetailContactMaterial) -> Result<Self, String> {
        let index = index_archive(&archive)?;
        let locs: Vec<ClusterLoc> = index.clusters.iter().map(|(l, _)| *l).collect();
        let infos: Vec<ClusterInfo> = index.clusters.iter().map(|(_, i)| *i).collect();
        Ok(Self::from_parts(&infos, Arc::new(ArchiveSource::new(archive, locs)), broadphase_limits(index.widest)?, material))
    }

    pub(crate) fn from_parts(infos: &[ClusterInfo], source: Arc<dyn ClusterSource>, limits: (f32, f32), material: RetailContactMaterial) -> Self {
        let clusters = infos
            .iter()
            .map(|i| ClusterEntry {
                bounds: Bounds { min: Vector3::new(i.min[0], i.min[1], i.min[2]), max: Vector3::new(i.max[0], i.max[1], i.max[2]) },
                triangles: i.triangles,
                params: i.params,
            })
            .collect();
        Self { source, cache: Default::default(), clusters, material, limits }
    }

    /// The query meshes of one cluster (runs of one group in cluster order), decoded.
    fn decode_runs(&self, entry: &ClusterEntry, bytes: &[u8]) -> Result<Arc<Vec<Run>>, String> {
        let cluster = decode_cluster_bytes(bytes, entry.params)?;
        // Runs of one group: preserve cluster order and partition further only for group
        // filters (as `skate_world::retail_collision_world` does).
        let mut runs = Vec::new();
        let mut cursor = 0;
        while cursor < cluster.len() {
            let group = cluster[cursor].group;
            // L1e (memory): the run's length first, so its buffers are allocated once at their size
            // (growing them left a trail of freed blocks of every size behind each window build,
            // and the Vita's heap kept growing around them).
            let len = cluster[cursor..].iter().take_while(|t| t.group == group).count();
            let mut triangles = Vec::with_capacity(len);
            let mut surfaces = Vec::with_capacity(len);
            while cursor < cluster.len() && cluster[cursor].group == group {
                let source = cluster[cursor];
                let (flags, cosines) = match source.edges {
                    Some(edges) => {
                        let (mut flags, cosines) = decode_native_edges(edges)?;
                        flags &= !TriangleFeature::ONE_SIDED;
                        if source.one_sided {
                            flags |= TriangleFeature::ONE_SIDED;
                        }
                        (flags, cosines)
                    }
                    None => (0x1e1, [-1.; 3]),
                };
                triangles.push(
                    WorldTriangle::from_vertices(
                        source.points.map(|p| Vector3::new(p[0], p[1], p[2])),
                        self.material,
                        u32::from(source.surface),
                        flags,
                        cosines,
                        0.,
                    )
                    .ok_or("Invalid RWCM collision triangle")?,
                );
                surfaces.push(source.surface);
                cursor += 1;
            }
            let bounds = Bounds::from_points(triangles.iter().flat_map(|t| t.triangle.vertices)).ok_or("Invalid RWCM cluster bounds")?;
            let triangle_bounds: Vec<Bounds> = triangles.iter().map(skate_core::physics::board_world::triangle_bounds_of).collect();
            let tree = skate_core::physics::board_world::MeshTree::build(&triangle_bounds);
            runs.push(Run { triangles, surfaces, bounds, group: i32::from(group), triangle_bounds, tree });
        }
        Ok(Arc::new(runs))
    }

    /// The world of the query meshes whose bounds meet a region (all of them for `None`), in
    /// archive order. Window builds keep the clusters they decoded: the next window, a few
    /// metres on, mostly holds the same ones (a build is memory- and allocation-bound on the
    /// Vita, and decoding was most of it).
    pub(crate) fn world(&self, regions: Option<&[Bounds]>) -> Result<BoardWorld, String> {
        self.build(regions, true)
    }

    /// The same without touching the cache (a world around a far point, built by the simulation).
    pub(crate) fn world_uncached(&self, regions: Option<&[Bounds]>) -> Result<BoardWorld, String> {
        self.build(regions, false)
    }

    fn build(&self, regions: Option<&[Bounds]>, cached: bool) -> Result<BoardWorld, String> {
        let t0 = std::time::Instant::now();
        let ids: Vec<usize> = (0..self.clusters.len())
            .filter(|&i| regions.is_none_or(|r| r.iter().any(|r| self.clusters[i].bounds.overlaps(*r))))
            .collect();
        let mut local = HashMap::new();
        let mut guard = if cached { Some(self.cache.lock().map_err(|_| "window cache lock poisoned".to_string())?) } else { None };
        let cache: &mut HashMap<usize, Arc<Vec<Run>>> = match guard.as_deref_mut() {
            Some(c) => c,
            None => &mut local,
        };
        let missing: Vec<usize> = ids.iter().copied().filter(|i| !cache.contains_key(i)).collect();
        let bytes = self.source.read(&missing)?;
        let t_read = t0.elapsed();
        for (id, bytes) in missing.iter().zip(&bytes) {
            let runs = self.decode_runs(&self.clusters[*id], bytes)?;
            cache.insert(*id, runs);
        }
        let selected: Vec<Arc<Vec<Run>>> = ids.iter().map(|i| cache[i].clone()).collect();
        if cached {
            // Keep only this window's clusters.
            let keep: std::collections::HashSet<usize> = ids.iter().copied().collect();
            cache.retain(|id, _| keep.contains(id));
        }
        // Exact capacity: a growing Vec would hold the old and the new buffer at once (a
        // 120,000-triangle window is 29 MB: that doubling is what exhausted the Vita's heap).
        // A retired window's buffers are refilled when they are big enough: after the first
        // windows no new 10 MB block is needed, which a fragmented heap may not have (the Vita
        // with the retail audio). A fresh buffer gets a quarter more room for the next windows.
        let upper: usize = ids.iter().map(|&i| self.clusters[i].triangles as usize).sum();
        let skate_core::physics::board_world::WorldBuffers { triangles, triangle_bounds, packed_surfaces } = take_buffers(upper);
        let room = upper + upper / 4;
        let fresh = triangles.capacity() < upper || packed_surfaces.capacity() < upper || triangle_bounds.capacity() < upper;
        let t_buffers = t0.elapsed();
        let (triangles, packed_surfaces, triangle_bounds) = if fresh {
            // No kept set holds this world (or none is free): a fresh one; the small set goes
            // back for later.
            if triangles.capacity() > 0 {
                if let Ok(mut pool) = RECYCLED.lock() {
                    pool.push(skate_core::physics::board_world::WorldBuffers { triangles, triangle_bounds, packed_surfaces });
                }
            }
            (Vec::with_capacity(room), Vec::with_capacity(room), Vec::with_capacity(room))
        } else {
            (triangles, packed_surfaces, triangle_bounds)
        };
        let (mut triangles, mut packed_surfaces, mut triangle_bounds) = (triangles, packed_surfaces, triangle_bounds);
        triangle_bounds.clear();
        let mut meshes = Vec::new();
        let mut trees = Vec::new();
        for runs in &selected {
            for run in runs.iter() {
                if regions.is_some_and(|r| !r.iter().any(|r| run.bounds.overlaps(*r))) {
                    continue;
                }
                let start = triangles.len();
                triangles.extend_from_slice(&run.triangles);
                packed_surfaces.extend_from_slice(&run.surfaces);
                triangle_bounds.extend_from_slice(&run.triangle_bounds);
                trees.push(run.tree.clone());
                meshes.push(QueryMesh {
                    geometry: 0,
                    rejection_flags: 0,
                    triangle_range: start..triangles.len(),
                    local_to_world: RetailAffineTransform::IDENTITY,
                    world_to_local: RetailAffineTransform::IDENTITY,
                    local_bounds: run.bounds,
                    matching_group: run.group,
                    pool: QueryPool::Ground,
                });
            }
        }
        let t_decode = t0.elapsed();
        let world = BoardWorld::with_query_metadata_prebuilt(
            triangles,
            QueryMetadata { packed_surfaces, meshes, static_edges: vec![], island_flags: 0 },
            triangle_bounds,
            trees,
        )
        .map_err(str::to_owned)?;
        // skate_vita: on the device with the `stage-profile` feature too (no environment there).
        if cfg!(target_os = "vita") {
            println!("WINDOW_PHASES read {:.1} ms ({} clusters), decode and copy {:.1} ms, index {:.1} ms", t_read.as_secs_f32() * 1000.0, missing.len(), (t_decode - t_read).as_secs_f32() * 1000.0, (t0.elapsed() - t_decode).as_secs_f32() * 1000.0);
        } else if cfg!(feature = "stage-profile") || std::env::var_os("WINDOW_PHASES").is_some() {
            eprintln!("WINDOW_PHASES read {:.1} ms, decode {:.1} ms (buffers at {:.1} ms, {}), index {:.1} ms ({} triangles, {} meshes, {} clusters decoded of {})", t_read.as_secs_f32() * 1000.0, (t_decode - t_read).as_secs_f32() * 1000.0, t_buffers.as_secs_f32() * 1000.0, if fresh { "fresh" } else { "reused" }, (t0.elapsed() - t_decode).as_secs_f32() * 1000.0, world.triangles().len(), world.query_metadata().map_or(0, |m| m.meshes.len()), missing.len(), ids.len());
        }
        // A window pads its queries as the whole world does (the limits of the whole set).
        Ok(if regions.is_some() { world.with_broadphase_limits(self.limits) } else { world })
    }
}

/// The streamed world of a session: the decoded set and where the current window was
/// built.
pub(crate) struct Window {
    set: std::sync::Arc<ClusterSet>,
    half: f32,
    trigger: f32,
    /// The foci the current window was built around (the deck and the skater).
    centres: Vec<[f32; 3]>,
    /// A window being built on another thread (device): its foci and the receiver.
    pending: Option<(Vec<[f32; 3]>, std::sync::mpsc::Receiver<Result<(BoardWorld, f32), String>>)>,
    /// Windows built so far, the last and the slowest build in ms, and the triangles in the
    /// current window (diagnostics for the device).
    pub(crate) builds: u32,
    pub(crate) last_ms: f32,
    pub(crate) max_ms: f32,
    pub(crate) triangles: usize,
    /// Times a tick had to wait for the background build.
    pub(crate) waits: u32,
    /// Where a replaced window goes to be freed: dropping 100,000 triangles and their indices
    /// inside a tick cost the simulation several milliseconds (dropped ticks on the device).
    retired: Option<std::sync::mpsc::Sender<BoardWorld>>,
}

/// skate_vita: a build still running when the session ends gives its buffers back before the
/// next session loads (it would otherwise hold a set the next district needs).
impl Drop for Window {
    fn drop(&mut self) {
        if let Some((_, rx)) = self.pending.take() {
            if let Ok(Ok((world, _))) = rx.recv() {
                recycle(world);
            }
        }
    }
}

/// How far inside the window the skater and the deck must be for the sim to keep running
/// on it while the next window is being built: the farthest any query of a tick reaches is
/// well below this (stage 1: exact with a 56 m margin under every scenario).
const SAFE_REACH: f32 = 40.0;
/// skate_vita: a background window is built around where the foci will be this many seconds on (their velocity), at
/// most `MAX_LEAD` metres ahead, so a fast skater reaches the new window's middle instead of the old one's edge (at
/// 20 m/s a build of 0.5-1 s used to end with the skater past the safe zone, and the tick waited 0.3-0.65 s).
const LEAD_SECONDS: f32 = 1.5;
/// Below `half - SAFE_REACH` (20 m): the foci are still safe in the window built for their lead.
const MAX_LEAD: f32 = 14.0;

/// The horizontal offset a background window is built ahead by (`LEAD_SECONDS`, at most `MAX_LEAD`).
fn lead(velocity: [f32; 3]) -> [f32; 3] {
    let (x, z) = (velocity[0] * LEAD_SECONDS, velocity[2] * LEAD_SECONDS);
    let length = (x * x + z * z).sqrt();
    if !length.is_finite() {
        return [0.0; 3];
    }
    let scale = if length > MAX_LEAD { MAX_LEAD / length } else { 1.0 };
    [x * scale, 0.0, z * scale]
}

#[cfg(test)]
mod lead_tests {
    #[test]
    fn the_lead_follows_the_velocity_up_to_its_cap() {
        assert_eq!(super::lead([4.0, -9.0, 0.0]), [6.0, 0.0, 0.0]);
        let fast = super::lead([0.0, 0.0, 30.0]);
        assert!((fast[2] - super::MAX_LEAD).abs() < 1e-4 && fast[0] == 0.0);
        assert_eq!(super::lead([f32::NAN, 0.0, 1.0]), [0.0; 3]);
    }
}

impl Window {
    /// A window over an RWCM archive in memory.
    pub(crate) fn from_archive(archive: Vec<u8>, material: RetailContactMaterial, half: f32, trigger: f32, at: [f32; 3]) -> Result<(Self, BoardWorld), String> {
        Self::over(ClusterSet::from_archive(archive, material)?, half, trigger, at)
    }

    /// A window over the district collision installed for this session.
    pub(crate) fn from_installed(material: RetailContactMaterial, half: f32, trigger: f32, at: [f32; 3]) -> Option<Result<(Self, BoardWorld), String>> {
        let d = INSTALLED.lock().unwrap().take()?;
        Some(Self::over(ClusterSet::from_parts(&d.clusters, d.source, d.limits, material), half, trigger, at))
    }

    fn over(set: ClusterSet, half: f32, trigger: f32, at: [f32; 3]) -> Result<(Self, BoardWorld), String> {
        let set = std::sync::Arc::new(set);
        let mut window = Self { set, half, trigger, centres: Vec::new(), pending: None, builds: 0, last_ms: 0.0, max_ms: 0.0, triangles: 0, waits: 0, retired: None };
        // The first window is built around the spawn for both foci (deck and skater).
        let world = window.build(&[at, at])?;
        Ok((window, world))
    }

    fn regions(&self, foci: &[[f32; 3]]) -> Vec<Bounds> {
        let h = self.half;
        foci.iter()
            .map(|at| Bounds {
                min: Vector3::new(at[0] - h, f32::NEG_INFINITY, at[2] - h),
                max: Vector3::new(at[0] + h, f32::INFINITY, at[2] + h),
            })
            .collect()
    }

    fn built(&mut self, foci: &[[f32; 3]], world: &BoardWorld, ms: f32) {
        self.centres = foci.to_vec();
        self.builds += 1;
        self.last_ms = ms;
        self.max_ms = self.max_ms.max(ms);
        self.triangles = world.triangles().len();
    }

    fn build(&mut self, foci: &[[f32; 3]]) -> Result<BoardWorld, String> {
        let started = std::time::Instant::now();
        let world = self.set.world(Some(&self.regions(foci)))?;
        self.built(foci, &world, started.elapsed().as_secs_f32() * 1000.0);
        Ok(world)
    }

    /// Frees a replaced window on another thread (the builder's kind), not in the tick.
    fn retire(&mut self, old: BoardWorld) {
        if !BACKGROUND.load(Ordering::Relaxed) {
            return;
        }
        if self.retired.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<BoardWorld>();
            let init = *INIT.lock().unwrap();
            let spawned = std::thread::Builder::new().name("window-free".into()).stack_size(256 * 1024).spawn(move || {
                if let Some(init) = init {
                    init();
                }
                while let Ok(world) = rx.recv() {
                    // The buffers go back for the next window.
                    recycle(world);
                }
            });
            if spawned.is_ok() {
                self.retired = Some(tx);
            }
        }
        if let Some(tx) = &self.retired {
            let _ = tx.send(old);
        }
    }

    /// A transient world around a point far from the skater (respawn candidates, marker
    /// returns): the same clusters as the whole world has there.
    pub(crate) fn around(&self, at: [f32; 3]) -> Result<BoardWorld, String> {
        self.set.world_uncached(Some(&self.regions(&[at])))
    }

    /// Whether the current window holds everything within `reach` metres of `at`: the point
    /// is that far inside the box around one of the foci the window was built for.
    pub(crate) fn covers(&self, at: [f32; 3], reach: f32) -> bool {
        let inner = self.half - reach;
        self.centres.iter().any(|c| (at[0] - c[0]).abs().max((at[2] - c[2]).abs()) <= inner)
    }

    /// Whether a focus is `trigger` metres (horizontally) from where the window was built
    /// around it.
    fn moved(&self, foci: &[[f32; 3]]) -> bool {
        foci.len() != self.centres.len()
            || foci.iter().zip(&self.centres).any(|(a, c)| (a[0] - c[0]).abs().max((a[2] - c[2]).abs()) >= self.trigger)
    }

    /// Keeps `world` the window for these foci (the deck and the skater, which may be far
    /// apart: a bailed board flung away; the window then holds both places).
    ///
    /// With a background builder (`configure_background`) the new window is built on another
    /// thread and swapped in between ticks; the old one stays in use meanwhile, and the tick
    /// waits only if the foci are no longer safely inside it. Without one the old window
    /// goes first and the new one is built here (the desktop oracle: deterministic).
    pub(crate) fn follow(&mut self, foci: &[[f32; 3]], velocity: [f32; 3], world: &mut BoardWorld) -> Result<(), String> {
        let background = BACKGROUND.load(Ordering::Relaxed);
        // Where the windows are built around: ahead of the foci on the device (`LEAD_SECONDS`), the foci themselves
        // in the tick (the desktop oracle: its windows stay where they were).
        let lead = if background { lead(velocity) } else { [0.0; 3] };
        let aims: Vec<[f32; 3]> = foci.iter().map(|f| [f[0] + lead[0], f[1], f[2] + lead[2]]).collect();
        // At most two rounds: a window that arrives may be for foci the skater has since
        // left (a teleport), and the next one is then waited for.
        for _ in 0..2 {
            if let Some((_, rx)) = &self.pending {
                let safe = foci.iter().all(|f| self.covers(*f, SAFE_REACH));
                let ready = match rx.try_recv() {
                    Ok(r) => r,
                    Err(std::sync::mpsc::TryRecvError::Empty) if safe => return Ok(()),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        self.waits += 1;
                        rx.recv().map_err(|_| "the window builder stopped".to_string())?
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return Err("the window builder stopped".into()),
                };
                let (fresh, ms) = ready?;
                let (new_foci, _) = self.pending.take().unwrap();
                self.built(&new_foci, &fresh, ms);
                let old = std::mem::replace(world, fresh);
                self.retire(old);
            }
            if !self.moved(&aims) && foci.iter().all(|f| self.covers(*f, SAFE_REACH)) {
                return Ok(());
            }
            if !background {
                // The old window goes before the new one is built: two would not fit the heap.
                *world = BoardWorld::new(Vec::new());
                *world = self.build(foci)?;
                return Ok(());
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let (set, regions) = (self.set.clone(), self.regions(&aims));
            let init = *INIT.lock().unwrap();
            std::thread::Builder::new()
                .name("window-build".into())
                .stack_size(1024 * 1024)
                .spawn(move || {
                    if let Some(init) = init {
                        init();
                    }
                    let started = std::time::Instant::now();
                    // A session that ended meanwhile no longer receives: its buffers go back.
                    if let Err(std::sync::mpsc::SendError(Ok((world, _)))) = tx.send(set.world(Some(&regions)).map(|w| (w, started.elapsed().as_secs_f32() * 1000.0))) {
                        recycle(world);
                    }
                })
                .map_err(|e| e.to_string())?;
            self.pending = Some((aims.clone(), rx));
            // Near the old window the simulation goes on (the loop returns at the top); far
            // from it (a teleport) the new window is waited for.
            if foci.iter().all(|f| self.covers(*f, SAFE_REACH)) {
                return Ok(());
            }
        }
        Ok(())
    }
}
