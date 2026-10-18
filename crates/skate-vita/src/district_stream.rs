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

type Tile = (i32, i32);

/// One upload: a frame's budget is spent between them, so none of them is long.
enum Task {
    Texture(u32, Texture),
    Begin(Tile, usize),
    Mesh(Tile, usize, TileMesh),
    Finish(Tile, usize),
}

struct Loaded {
    tile: Tile,
    lod: usize,
    meshes: Vec<TileMesh>,
    textures: Vec<(u32, Texture)>,
}

/// What the loader reads: written by the main thread each frame.
#[derive(Default)]
struct Shared {
    /// Wanted (tile, level), nearest first.
    wanted: Vec<(Tile, usize)>,
    resident: HashSet<(Tile, usize)>,
    textures: HashSet<u32>,
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
    failure: Option<String>,
    frame: u64,
    started: Instant,
}

impl Default for Streamer {
    fn default() -> Self {
        Self { generation: u64::MAX, view: None, loader: None, uploaded: (0, 0), tasks: Default::default(), slowest: Duration::ZERO, failure: None, frame: 0, started: Instant::now() }
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

    /// Once per frame, before drawing.
    pub fn frame(&mut self, scene: &mut Scene, eye: Vec3, have_eye: bool) {
        self.frame += 1;
        let generation = DISTRICT_GENERATION.load(Ordering::Relaxed);
        if generation != self.generation {
            self.generation = generation;
            self.stop();
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

        // What is wanted: every tile of the district within the drawing radius, at its level.
        let mut wanted: Vec<(f32, Tile, usize)> = view
            .tiles
            .keys()
            .filter_map(|&t| {
                let d = tile_distance(t, view.tile_size, eye);
                (d < RADIUS).then_some((d, t, level(d)))
            })
            .collect();
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Uploads, within a budget.
        let started = Instant::now();
        loop {
            if self.tasks.is_empty() {
                match loader.results.try_recv() {
                    Ok(Ok(l)) => {
                        self.tasks.extend(l.textures.into_iter().map(|(id, t)| Task::Texture(id, t)));
                        self.tasks.push_back(Task::Begin(l.tile, l.lod));
                        self.tasks.extend(l.meshes.into_iter().map(|m| Task::Mesh(l.tile, l.lod, m)));
                        self.tasks.push_back(Task::Finish(l.tile, l.lod));
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

        // Drops: tiles beyond the radius, and levels the wanted one has replaced.
        let want: HashMap<Tile, usize> = wanted.iter().map(|&(_, t, l)| (t, l)).collect();
        for (tile, lod) in gpu.resident() {
            let far = tile_distance(tile, view.tile_size, eye) > RADIUS + KEEP;
            let replaced = want.get(&tile).is_some_and(|&w| w != lod && gpu.has_tile(tile, w));
            if far || replaced {
                gpu.drop_tile(tile, lod);
            }
        }
        gpu.trim_textures(TEXTURE_BUDGET);

        // What the loader sees.
        let mut shared = loader.shared.lock().unwrap();
        shared.wanted = wanted.iter().map(|&(_, t, l)| (t, l)).collect();
        shared.resident = gpu.resident().into_iter().collect();
        shared.textures = gpu.texture_ids().into_iter().collect();
        drop(shared);

        if self.frame % 600 == 0 {
            println!(
                "district stream: {} tiles uploaded, {} textures; resident meshes {:.1} MiB, textures {:.1} MiB; slowest upload frame {:.1} ms ({} s)",
                self.uploaded.0,
                self.uploaded.1,
                gpu.mesh_bytes as f64 / 1048576.0,
                gpu.texture_bytes as f64 / 1048576.0,
                self.slowest.as_secs_f32() * 1000.0,
                self.started.elapsed().as_secs()
            );
            self.slowest = Duration::ZERO;
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
        let _ = vita_platform::vita::pin_current_thread(2);
        // Textures sent and not yet seen resident (the main thread uploads them next frame).
        let mut sent: HashSet<u32> = HashSet::new();
        while !stop2.load(Ordering::Relaxed) {
            let job = {
                let s = shared2.lock().unwrap();
                sent.retain(|t| !s.textures.contains(t));
                s.wanted.iter().find(|(t, l)| !s.resident.contains(&(*t, *l))).copied().map(|j| (j, s.textures.clone()))
            };
            let Some(((tile, lod), resident)) = job else {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            let result = load(&view, tile, lod, &resident, &mut sent);
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

fn load(view: &DistrictView, tile: Tile, lod: usize, resident: &HashSet<u32>, sent: &mut HashSet<u32>) -> Result<Loaded, String> {
    let entry = view.tiles.get(&tile).ok_or("no such tile")?;
    let bytes = view.file.read_tile(entry, lod)?;
    let meshes = if bytes.is_empty() { Vec::new() } else { decode_lod(&bytes)? };
    let mut textures = Vec::new();
    for mesh in &meshes {
        for d in &mesh.draws {
            for id in [d.albedo, d.lightmap] {
                if id != vita_pack::district::NO_TEXTURE && id != vita_pack::district::GREY_LIGHTMAP && !resident.contains(&id) && sent.insert(id) {
                    let at = *view.textures.get(id as usize).ok_or("texture id outside the table")?;
                    textures.push((id, view.file.read_texture(at)?));
                }
            }
        }
    }
    Ok(Loaded { tile, lod, meshes, textures })
}
