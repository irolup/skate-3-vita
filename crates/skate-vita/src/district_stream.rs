//! Streams a district's render tiles: each frame the main thread says which
//! tiles at which level of detail are wanted around the camera; a loader thread (core 2) reads
//! and decodes them (and the textures they need) from the district pack by offset; the main
//! thread uploads what has arrived within a time budget and drops what is no longer wanted.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glam::Vec3;
use vita_pack::district::{TileMesh, decode_lod};
use vita_pack::texture::Texture;
use vita_render::district::{LOD_RANGES, RADIUS};
use vita_render::scene::Scene;

use crate::packs::{DISTRICT, DISTRICT_GENERATION, DistrictView};

/// Textures kept on the GPU (the least recently used go first).
const TEXTURE_BUDGET: usize = 56 << 20;
/// Tiles are dropped this far (m) beyond the drawing radius.
const KEEP: f32 = 50.0;
/// Time the main thread spends uploading per frame.
const UPLOAD_BUDGET: Duration = Duration::from_millis(3);
/// Time the main thread spends dropping tile levels per frame (each level frees its meshes' GPU buffers; several
/// levels in one frame made it 16-27 ms in DownTown). At least one level goes each frame.
const DROP_BUDGET: Duration = Duration::from_millis(1);
/// The wanted tiles, the drops and the loader's view are worked out every this many frames (the uploads run every
/// frame): the camera moves a third of a metre a frame at speed, and the planning over the district's ~500 tiles
/// was a millisecond or more of every frame.
const PLAN_EVERY: u64 = 4;
/// Tiles are wanted and ranked by the nearer of the camera and where it will be this many seconds on (its smoothed
/// velocity), at most `MAX_AHEAD` metres ahead: at speed the tiles in front load before the camera reaches them.
const AHEAD_SECONDS: f32 = 1.5;
const MAX_AHEAD: f32 = 40.0;

type Tile = (i32, i32);

/// Wanted tiles of one distance band and how many of them are missing or at another level.
#[derive(Clone, Copy, Debug, Default)]
pub struct Coverage {
    pub wanted: u32,
    pub holes: u32,
    pub coarse: u32,
}

/// The loader's work since the last log line: tiles, card reads (count, bytes, µs), decoding µs.
static LOADS: AtomicU64 = AtomicU64::new(0);
static READS: AtomicU64 = AtomicU64::new(0);
static READ_BYTES: AtomicU64 = AtomicU64::new(0);
static READ_US: AtomicU64 = AtomicU64::new(0);
static READ_MAX_US: AtomicU64 = AtomicU64::new(0);
static DECODE_US: AtomicU64 = AtomicU64::new(0);
/// The same reads since the start (never reset): the streaming benchmark's per-leg counts.
static TOTAL_READS: AtomicU64 = AtomicU64::new(0);
static TOTAL_READ_BYTES: AtomicU64 = AtomicU64::new(0);
static TOTAL_READ_US: AtomicU64 = AtomicU64::new(0);

/// The loader's card reads since the start: (count, bytes, µs).
#[cfg_attr(not(feature = "stream-bench"), allow(dead_code))]
pub fn total_reads() -> (u64, u64, u64) {
    (TOTAL_READS.load(Ordering::Relaxed), TOTAL_READ_BYTES.load(Ordering::Relaxed), TOTAL_READ_US.load(Ordering::Relaxed))
}

fn timed_read<T>(bytes: usize, read: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let out = read();
    let us = started.elapsed().as_micros() as u64;
    READS.fetch_add(1, Ordering::Relaxed);
    READ_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    READ_US.fetch_add(us, Ordering::Relaxed);
    READ_MAX_US.fetch_max(us, Ordering::Relaxed);
    TOTAL_READS.fetch_add(1, Ordering::Relaxed);
    TOTAL_READ_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    TOTAL_READ_US.fetch_add(us, Ordering::Relaxed);
    out
}

/// One upload: a frame's budget is spent between them, so none of them is long.
enum Task {
    Texture(u32, Texture),
    Begin(Tile, usize),
    Mesh(Tile, usize, TileMesh),
    Finish(Tile, usize),
}

/// One tile's levels read together (one, or the middle and far ones in one read) and the textures they need.
struct Loaded {
    tile: Tile,
    levels: Vec<(usize, Vec<TileMesh>)>,
    textures: Vec<(u32, Texture)>,
}

/// What the loader reads: written by the main thread each frame.
#[derive(Default)]
struct Shared {
    /// Wanted (tile, level), nearest first.
    wanted: Vec<(Tile, usize)>,
    resident: HashSet<(Tile, usize)>,
    textures: HashSet<u32>,
    /// The middle and far levels read together (see `Streamer::lean`).
    merge: bool,
}

struct Loader {
    shared: Arc<Mutex<Shared>>,
    results: Receiver<Result<Loaded, String>>,
    stop: Arc<AtomicBool>,
}

pub struct Streamer {
    generation: u64,
    view: Option<Arc<DistrictView>>,
    loader: Option<Loader>,
    /// Counts for the log: tiles and textures uploaded, bytes, slowest frame of uploads.
    uploaded: (u32, u32),
    tasks: std::collections::VecDeque<Task>,
    slowest: Duration,
    /// The slowest frame's drops and the rest of the frame's work (wanted list, loader state), for the log.
    slowest_drops: Duration,
    slowest_rest: Duration,
    failure: Option<String>,
    frame: u64,
    started: Instant,
    /// After the last frame's uploads, by distance band from the camera (`LOD_RANGES`, then `RADIUS`): the wanted
    /// tiles, those with no level resident (holes) and those at another level than wanted (coarse).
    coverage: [Coverage; 3],
    /// The camera's last position and its smoothed horizontal velocity (m/s).
    last_eye: Option<(Vec3, Instant)>,
    velocity: Vec3,
    /// Off: tiles are wanted around the camera only (the streaming benchmark's comparison).
    pub look_ahead: bool,
    /// On: a tile's middle and far levels come in one read and stay while it is near (off: each level is read when
    /// wanted and the others dropped; the streaming benchmark's comparison).
    pub lean: bool,
    /// The wanted tiles (distance, tile, level), nearest first, as of the last planning frame.
    wanted: Vec<(f32, Tile, usize)>,
    /// For the log, since its last line: frames with a hole per distance band, frames whose draw skipped a piece
    /// waiting for its texture, and the most pieces skipped in one frame.
    hole_frames: [u32; 3],
    wait_frames: u32,
    max_waits: u32,
}

impl Default for Streamer {
    fn default() -> Self {
        Self { generation: u64::MAX, view: None, loader: None, uploaded: (0, 0), tasks: Default::default(), slowest: Duration::ZERO, slowest_drops: Duration::ZERO, slowest_rest: Duration::ZERO, failure: None, frame: 0, started: Instant::now(), coverage: [Coverage::default(); 3], last_eye: None, velocity: Vec3::ZERO, look_ahead: true, lean: true, wanted: Vec::new(), hole_frames: [0; 3], wait_frames: 0, max_waits: 0 }
    }
}

fn level(distance: f32) -> usize {
    if distance < LOD_RANGES[0] {
        0
    } else if distance < LOD_RANGES[1] {
        1
    } else {
        2
    }
}

fn tile_distance(tile: Tile, size: f32, eye: Vec3) -> f32 {
    let (x0, z0) = (tile.0 as f32 * size, tile.1 as f32 * size);
    let dx = (x0 - eye.x).max(eye.x - (x0 + size)).max(0.0);
    let dz = (z0 - eye.z).max(eye.z - (z0 + size)).max(0.0);
    (dx * dx + dz * dz).sqrt()
}

impl Streamer {
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn active(&self) -> bool {
        self.view.is_some()
    }

    /// Drops every tile and texture and starts the district again on the next frame (the streaming benchmark: each
    /// leg starts from nothing).
    #[cfg_attr(not(feature = "stream-bench"), allow(dead_code))]
    pub fn restart(&mut self) {
        self.generation = u64::MAX;
    }

    /// The tiles after the last frame's uploads by distance band; see `coverage`.
    #[cfg_attr(not(feature = "stream-bench"), allow(dead_code))]
    pub fn coverage(&self) -> [Coverage; 3] {
        self.coverage
    }

    /// Once per frame, before drawing.
    pub fn frame(&mut self, scene: &mut Scene, eye: Vec3, have_eye: bool) {
        self.frame += 1;
        let generation = DISTRICT_GENERATION.load(Ordering::Relaxed);
        if generation != self.generation {
            self.generation = generation;
            self.stop();
            // Uploads queued for the previous district (or start) go with it.
            self.tasks.clear();
            self.wanted.clear();
            self.last_eye = None;
            self.view = DISTRICT.lock().unwrap().clone();
            self.failure = None;
            match &self.view {
                Some(view) => {
                    scene.load_district(view.tile_size, view.horizon, view.exposure);
                    self.loader = Some(spawn_loader(view.clone()));
                    println!("district stream: {} started", view.name);
                }
                None => scene.unload_district(),
            }
        }
        let (Some(view), Some(loader)) = (&self.view, &self.loader) else { return };
        let Some(gpu) = scene.district_mut() else { return };
        if !have_eye {
            return;
        }

        // Where the camera is going: its velocity over the last frames (a cut or a teleport resets it).
        let now = Instant::now();
        if let Some((last, at)) = self.last_eye {
            let dt = now.duration_since(at).as_secs_f32();
            let step = Vec3::new(eye.x - last.x, 0.0, eye.z - last.z);
            if dt > 0.0 && dt < 0.25 && step.length() < 30.0 {
                self.velocity = self.velocity.lerp(step / dt, (dt * 4.0).min(1.0));
            } else {
                self.velocity = Vec3::ZERO;
            }
        }
        self.last_eye = Some((eye, now));
        let ahead = if self.look_ahead { eye + (self.velocity * AHEAD_SECONDS).clamp_length_max(MAX_AHEAD) } else { eye };

        // What is wanted: every tile of the district within the drawing radius of the camera or of where it is
        // going, at the level of the nearer.
        let plan = self.frame % PLAN_EVERY == 0 || self.wanted.is_empty();
        if plan {
            self.wanted.clear();
            self.wanted.extend(view.tiles.keys().filter_map(|&t| {
                let d = tile_distance(t, view.tile_size, eye).min(tile_distance(t, view.tile_size, ahead));
                (d < RADIUS).then_some((d, t, level(d)))
            }));
            self.wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        let wanted = std::mem::take(&mut self.wanted);

        // Uploads, within a budget.
        let started = Instant::now();
        loop {
            if self.tasks.is_empty() {
                match loader.results.try_recv() {
                    Ok(Ok(l)) => {
                        self.tasks.extend(l.textures.into_iter().map(|(id, t)| Task::Texture(id, t)));
                        for (lod, meshes) in l.levels {
                            self.tasks.push_back(Task::Begin(l.tile, lod));
                            self.tasks.extend(meshes.into_iter().map(|m| Task::Mesh(l.tile, lod, m)));
                            self.tasks.push_back(Task::Finish(l.tile, lod));
                        }
                    }
                    Ok(Err(e)) => {
                        println!("district stream error: {e}");
                        self.failure = Some(e);
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            match self.tasks.pop_front() {
                Some(Task::Texture(id, t)) => {
                    gpu.add_texture(id, &t);
                    self.uploaded.1 += 1;
                }
                Some(Task::Begin(tile, lod)) => gpu.begin_tile(tile, lod),
                Some(Task::Mesh(tile, lod, m)) => gpu.add_mesh(tile, lod, &m),
                Some(Task::Finish(tile, lod)) => {
                    gpu.finish_tile(tile, lod);
                    self.uploaded.0 += 1;
                }
                None => {}
            }
            if started.elapsed() >= UPLOAD_BUDGET {
                break;
            }
        }
        self.slowest = self.slowest.max(started.elapsed());
        // Measured around the camera itself (not where it is going), so runs with and without the look-ahead compare.
        if plan {
            self.coverage = [Coverage::default(); 3];
            for &(_, t, _) in &wanted {
                let d = tile_distance(t, view.tile_size, eye);
                if d >= RADIUS {
                    continue;
                }
                let c = &mut self.coverage[level(d)];
                let any = (0..3).any(|i| gpu.has_tile(t, i));
                c.wanted += 1;
                c.holes += u32::from(!any);
                // Coarse: drawn, but not at the camera's level or a finer one.
                c.coarse += u32::from(any && !(0..=level(d)).any(|i| gpu.has_tile(t, i)));
            }
        }
        for (frames, c) in self.hole_frames.iter_mut().zip(&self.coverage) {
            *frames += u32::from(c.holes > 0);
        }
        // The last frame's draw.
        self.wait_frames += u32::from(gpu.texture_waits > 0);
        self.max_waits = self.max_waits.max(gpu.texture_waits);

        // Drops: tiles beyond the radius, and the near level beyond the middle level's range. The coarser levels
        // stay while a tile is near (a few MB of GPU memory): leaving a tile then reads nothing, where each level it
        // went back to was a card read (the card's small reads were what held the tiles up at speed).
        // Within `DROP_BUDGET`, farthest first; what is left goes on the next frames.
        let drops_started = Instant::now();
        if plan {
            let want: HashMap<Tile, usize> = wanted.iter().map(|&(_, t, l)| (t, l)).collect();
            let mut drops: Vec<(f32, Tile, usize)> = gpu
                .resident()
                .into_iter()
                .filter_map(|(tile, lod)| {
                    let d = tile_distance(tile, view.tile_size, eye);
                    let far = d > RADIUS + KEEP;
                    let stale_near = lod == 0 && d > LOD_RANGES[1] && want.get(&tile).is_none_or(|&w| w > 0 && gpu.has_tile(tile, w));
                    let replaced = !self.lean && want.get(&tile).is_some_and(|&w| w != lod && gpu.has_tile(tile, w));
                    (far || (if self.lean { stale_near } else { replaced })).then_some((d, tile, lod))
                })
                .collect();
            drops.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, tile, lod) in drops {
                gpu.drop_tile(tile, lod);
                if drops_started.elapsed() >= DROP_BUDGET {
                    break;
                }
            }
            gpu.trim_textures(TEXTURE_BUDGET);
        }
        let drops_took = drops_started.elapsed();

        // What the loader sees.
        if plan {
            let mut shared = loader.shared.lock().unwrap();
            shared.wanted = wanted.iter().map(|&(_, t, l)| (t, l)).collect();
            shared.resident = gpu.resident().into_iter().collect();
            shared.textures = gpu.texture_ids().into_iter().collect();
            shared.merge = self.lean;
        }
        self.wanted = wanted;
        let rest = started.elapsed().saturating_sub(drops_took);
        if drops_took + rest > self.slowest_drops + self.slowest_rest {
            (self.slowest_drops, self.slowest_rest) = (drops_took, rest);
        }

        if self.frame % 600 == 0 {
            println!(
                "district stream: {} tiles uploaded, {} textures; resident meshes {:.1} MiB, textures {:.1} MiB; slowest upload frame {:.1} ms ({} s); slowest streamer frame: drops {:.1} ms, uploads and the rest {:.1} ms; frames with holes <90 m {}, 90-160 m {}, 160-250 m {}; frames waiting for a texture {} (at most {} pieces); queued uploads {}; loader: {} tiles, {} reads of {:.1} MiB in {:.0} ms (slowest {:.0} ms), decoding {:.0} ms",
                self.uploaded.0,
                self.uploaded.1,
                gpu.mesh_bytes as f64 / 1048576.0,
                gpu.texture_bytes as f64 / 1048576.0,
                self.slowest.as_secs_f32() * 1000.0,
                self.started.elapsed().as_secs(),
                self.slowest_drops.as_secs_f32() * 1000.0,
                self.slowest_rest.as_secs_f32() * 1000.0,
                self.hole_frames[0],
                self.hole_frames[1],
                self.hole_frames[2],
                self.wait_frames,
                self.max_waits,
                self.tasks.len(),
                LOADS.swap(0, Ordering::Relaxed),
                READS.swap(0, Ordering::Relaxed),
                READ_BYTES.swap(0, Ordering::Relaxed) as f64 / 1048576.0,
                READ_US.swap(0, Ordering::Relaxed) as f64 / 1000.0,
                READ_MAX_US.swap(0, Ordering::Relaxed) as f64 / 1000.0,
                DECODE_US.swap(0, Ordering::Relaxed) as f64 / 1000.0
            );
            // Every card read of the last 10 s by source, busiest first (who the tiles share the card with).
            let mut card: Vec<String> = skate_data::io_gate::take_accounts()
                .iter()
                .map(|(s, [n, bytes, us, max])| format!("{s} {n}x {:.1} MiB {:.0} ms (max {:.0})", *bytes as f64 / 1048576.0, *us as f64 / 1000.0, *max as f64 / 1000.0))
                .collect();
            let [n, bytes, us, max] = skate_audio::stream::take_reads();
            card.push(format!("audio streams {n}x {:.1} MiB {:.0} ms (max {:.0})", bytes as f64 / 1048576.0, us as f64 / 1000.0, max as f64 / 1000.0));
            println!("CARD reads in 10 s: {}", card.join("; "));
            self.slowest = Duration::ZERO;
            (self.slowest_drops, self.slowest_rest) = (Duration::ZERO, Duration::ZERO);
            (self.hole_frames, self.wait_frames, self.max_waits) = ([0; 3], 0, 0);
        }
    }

    fn stop(&mut self) {
        if let Some(loader) = self.loader.take() {
            loader.stop.store(true, Ordering::Relaxed);
        }
    }
}

static LOADER_ID: AtomicU64 = AtomicU64::new(0);

fn spawn_loader(view: Arc<DistrictView>) -> Loader {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let (tx, results) = sync_channel(3);
    let stop = Arc::new(AtomicBool::new(false));
    let (shared2, stop2) = (shared.clone(), stop.clone());
    let id = LOADER_ID.fetch_add(1, Ordering::Relaxed);
    let _ = std::thread::Builder::new().name(format!("tile-load-{id}")).stack_size(1 << 20).spawn(move || {
        crate::alloc::tag_thread("tile");
        // Core 2 alone left it starved where the retail audio's render takes most of that core (20 voices on
        // Industrial's roads: decoding a tile 20x slower, the tiles late). On core 0 too, just below the main thread:
        // it takes the time the main thread waits for the display.
        let _ = vita_platform::vita::allow_current_thread_on(&[0, 2]);
        vita_platform::vita::set_current_thread_priority(165);
        // Textures sent and not yet seen resident (the main thread uploads them next frame).
        let mut sent: HashSet<u32> = HashSet::new();
        // Levels sent and not yet seen resident: not read again meanwhile (they were, while their uploads waited).
        let mut in_flight: HashSet<(Tile, usize)> = HashSet::new();
        while !stop2.load(Ordering::Relaxed) {
            let job = {
                let s = shared2.lock().unwrap();
                sent.retain(|t| !s.textures.contains(t));
                in_flight.retain(|j| !s.resident.contains(j) && s.wanted.iter().any(|w| w.0 == j.0));
                let pending = |j: &(Tile, usize)| s.resident.contains(j) || in_flight.contains(j);
                s.wanted.iter().find(|j| !pending(j)).copied().map(|(t, l)| {
                    // The middle and far levels lie side by side in the pack: one read brings both.
                    let partner = match l {
                        1 => Some(2),
                        2 => Some(1),
                        _ => None,
                    }
                    .filter(|p| s.merge && !pending(&(t, *p)));
                    ((t, l, partner), s.textures.clone())
                })
            };
            let Some(((tile, lod, partner), resident)) = job else {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            let result = load(&view, tile, lod, partner, &resident, &mut sent);
            if let Ok(l) = &result {
                in_flight.extend(l.levels.iter().map(|(lod, _)| (tile, *lod)));
            }
            // The queue holds three results; a full one waits (and notices a stop).
            let mut item = Some(result);
            while let Some(r) = item.take() {
                match tx.try_send(r) {
                    Ok(()) => {}
                    Err(std::sync::mpsc::TrySendError::Full(r)) => {
                        if stop2.load(Ordering::Relaxed) {
                            return;
                        }
                        item = Some(r);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return,
                }
            }
        }
    });
    Loader { shared, results, stop }
}

/// Reads a tile's level `lod`, and its `partner` level too when the two are adjacent in the pack (one read).
fn load(view: &DistrictView, tile: Tile, lod: usize, partner: Option<usize>, resident: &HashSet<u32>, sent: &mut HashSet<u32>) -> Result<Loaded, String> {
    let entry = view.tiles.get(&tile).ok_or("no such tile")?;
    let mut lods = vec![lod];
    if let Some(p) = partner {
        let (a, b) = (lod.min(p), lod.max(p));
        let (first, second) = (entry.lods[a], entry.lods[b]);
        if first.1 > 0 && second.1 > 0 && first.0 + u64::from(first.1) == second.0 {
            lods = vec![a, b];
        }
    }
    let (start, end) = (entry.lods[lods[0]].0, entry.lods[*lods.last().unwrap()].0 + u64::from(entry.lods[*lods.last().unwrap()].1));
    let bytes = if end > start { timed_read((end - start) as usize, || view.file.read_range_for("district tiles", start, (end - start) as u32))? } else { Vec::new() };
    let started = Instant::now();
    let mut levels = Vec::new();
    for &l in &lods {
        let (offset, size) = entry.lods[l];
        let at = (offset - start) as usize;
        let meshes = if size == 0 { Vec::new() } else { decode_lod(bytes.get(at..at + size as usize).ok_or("tile level outside its read")?)? };
        levels.push((l, meshes));
    }
    DECODE_US.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    LOADS.fetch_add(1, Ordering::Relaxed);
    // The textures the levels need that are not on the GPU or on their way, read together.
    let mut ids = Vec::new();
    for mesh in levels.iter().flat_map(|(_, m)| m) {
        for d in &mesh.draws {
            for id in [d.albedo, d.lightmap] {
                if id != vita_pack::district::NO_TEXTURE && id != vita_pack::district::GREY_LIGHTMAP && !resident.contains(&id) && sent.insert(id) {
                    ids.push(id);
                }
            }
        }
    }
    let entries: Vec<(u64, u32)> = ids.iter().map(|&id| view.textures.get(id as usize).copied().ok_or("texture id outside the table")).collect::<Result<_, _>>()?;
    let bytes: usize = entries.iter().map(|e| e.1 as usize).sum();
    let textures: Vec<(u32, Texture)> = if entries.is_empty() { Vec::new() } else { ids.into_iter().zip(timed_read(bytes, || view.file.read_textures(&entries))?).collect() };
    Ok(Loaded { tile, levels, textures })
}
