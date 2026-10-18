//! `vita-bake district-stats`: what a whole district costs, so the
//! choice between resident and streamed collision, and the tile budget, rest on numbers.
//!
//! Per district (`maps/<name>.skate` and its render-only `native-props/<name>.skate`):
//! file and collision sizes, collision triangles and clusters, rails and other records,
//! render triangles and textures (raw and DXT estimate), the 64 m render tiles (occupied
//! tiles, triangles and distinct texture bytes per tile) and the 32 m collision cells,
//! and the heap the real simulation uses to hold the district's collision (a counting
//! allocator around `Session::load_map`).
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use skate_data::retail_collision::visit_clusters;
use skate_data::skate_map::SkateMap;
use skate_sim::live::Session;
use vita_pack::map;

pub struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, layout, new) };
        if !q.is_null() {
            if new >= layout.size() {
                let now = CURRENT.fetch_add(new - layout.size(), Ordering::Relaxed) + new - layout.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                CURRENT.fetch_sub(layout.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

fn mib(n: usize) -> f64 {
    n as f64 / (1024.0 * 1024.0)
}

const TILE: f32 = 64.0;
const CELL: f32 = 32.0;

#[derive(Default)]
struct Bounds {
    lo: [f32; 3],
    hi: [f32; 3],
    set: bool,
}
impl Bounds {
    fn add(&mut self, p: [f32; 3]) {
        if !self.set {
            self.lo = p;
            self.hi = p;
            self.set = true;
        }
        for i in 0..3 {
            self.lo[i] = self.lo[i].min(p[i]);
            self.hi[i] = self.hi[i].max(p[i]);
        }
    }
}

/// DXT bytes of a texture capped at 256², with a mip chain (1/3 more), as the park bake
/// would store it (DXT1 8 bytes per 4×4, DXT5 16).
fn dxt_bytes(w: u32, h: u32, alpha: bool) -> usize {
    let (w, h) = (w.min(256).max(4) as usize, h.min(256).max(4) as usize);
    let base = (w / 4) * (h / 4) * if alpha { 16 } else { 8 };
    base + base / 3
}

fn percentile(v: &mut [usize], p: f64) -> usize {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p) as usize]
}

pub fn stats(install: &Path, names: &[String]) -> Result<(), String> {
    let assets = install.join("assets");
    println!("# District statistics\n");
    for name in names {
        println!("## {name}\n");
        let path = install.join("maps").join(format!("{name}.skate"));
        let file_bytes = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len() as usize;
        let source = SkateMap::load(&path)?;
        let props_path = assets.join("private/native-props").join(format!("{name}.skate"));
        let props = if props_path.is_file() {
            Some(SkateMap::parse_render_only(&std::fs::read(&props_path).map_err(|e| e.to_string())?)?)
        } else {
            None
        };
        println!("- source: {:.1} MiB; spawn ({:.1}, {:.1}, {:.1}); {} rails, {} doors, {} lights, {} routes; {} materials", mib(file_bytes), source.spawn[0], source.spawn[1], source.spawn[2], source.rails.len(), source.doors.len(), source.lights.len(), source.routes.len(), source.materials.len());
        let rail_points: usize = source.rails.iter().map(|r| r.points.len()).sum();
        let rail_native: usize = source.rails.iter().filter_map(|r| r.native.as_ref()).map(Vec::len).sum();
        println!("- rails: {rail_points} points, {:.1} KiB of native spline data", rail_native as f64 / 1024.0);
        println!("- props package: {}", if props.is_some() { "present" } else { "absent (no props converted)" });

        // Collision.
        let mut clusters = 0usize;
        let mut triangles = 0usize;
        let mut cells: BTreeMap<(i32, i32), usize> = BTreeMap::new();
        let mut bounds = Bounds::default();
        let mut meshes: BTreeSet<String> = BTreeSet::new();
        let rwcm: usize = match source.extensions.iter().find(|e| &e.tag == b"RWCM") {
            Some(e) => {
                visit_clusters(&e.payload, |mesh, tris| {
                    clusters += 1;
                    triangles += tris.len();
                    meshes.insert(mesh.to_owned());
                    for t in tris {
                        let c = [0, 1, 2].map(|i| (t.points[0][i] + t.points[1][i] + t.points[2][i]) / 3.0);
                        *cells.entry(((c[0] / CELL).floor() as i32, (c[2] / CELL).floor() as i32)).or_default() += 1;
                        for p in t.points {
                            bounds.add(p);
                        }
                    }
                    Ok(())
                })?;
                e.payload.len()
            }
            None => 0,
        };
        let portable = source.geometry.collision.len();
        println!(
            "- collision: RWCM {:.1} MiB, {} meshes, {} clusters, {} triangles ({} portable triangles); extent {:.0} × {:.0} × {:.0} m; {} occupied 32 m cells (max {} triangles in one)",
            mib(rwcm),
            meshes.len(),
            clusters,
            triangles,
            portable,
            bounds.hi[0] - bounds.lo[0],
            bounds.hi[1] - bounds.lo[1],
            bounds.hi[2] - bounds.lo[2],
            cells.len(),
            cells.values().max().copied().unwrap_or(0)
        );

        // The collision a window around the skater would hold (a square of side 2R, in
        // 32 m cells), centred on every occupied cell: the streamed-collision budget.
        let mut windows = String::new();
        for radius in [100.0_f32, 200.0, 300.0] {
            let reach = (radius / CELL).ceil() as i32;
            let mut counts: Vec<usize> = cells
                .keys()
                .map(|&(cx, cz)| {
                    let mut n = 0;
                    for dx in -reach..=reach {
                        for dz in -reach..=reach {
                            n += cells.get(&(cx + dx, cz + dz)).copied().unwrap_or(0);
                        }
                    }
                    n
                })
                .collect();
            let max = counts.iter().max().copied().unwrap_or(0);
            let p95 = percentile(&mut counts, 0.95);
            windows += &format!(" {radius:.0} m: p95 {p95}, max {max} ({:.0}% of all);", 100.0 * max as f64 / triangles.max(1) as f64);
        }
        println!("- collision triangles in a window of 2R m (centred on any occupied cell):{windows}");

        // Render: the map and its props, per 64 m tile.
        let sources: Vec<&SkateMap> = std::iter::once(&source).chain(props.as_ref()).collect();
        let mut tiles: BTreeMap<(i32, i32), (usize, BTreeSet<(usize, u32)>)> = BTreeMap::new();
        let mut render_tris = 0usize;
        let mut render_verts = 0usize;
        let mut textures_used: BTreeSet<(usize, u32)> = BTreeSet::new();
        for (s, src) in sources.iter().enumerate() {
            render_verts += src.geometry.vertices.len();
            for tri in src.geometry.indices.chunks_exact(3) {
                let v = [tri[0], tri[1], tri[2]].map(|i| &src.geometry.vertices[i as usize]);
                let c = [0, 1, 2].map(|i| (v[0].position[i] + v[1].position[i] + v[2].position[i]) / 3.0);
                let m = &src.materials[v[0].material as usize - 1];
                render_tris += 1;
                let tile = tiles.entry(((c[0] / TILE).floor() as i32, (c[2] / TILE).floor() as i32)).or_default();
                tile.0 += 1;
                for t in [m.textures[0], m.textures[1]] {
                    if t != 0 {
                        tile.1.insert((s, t));
                        textures_used.insert((s, t));
                    }
                }
            }
        }
        let alpha_albedo: BTreeSet<(usize, u32)> = sources
            .iter()
            .enumerate()
            .flat_map(|(s, src)| src.materials.iter().filter(|m| m.alpha_mode != 0 && m.textures[0] != 0).map(move |m| (s, m.textures[0])))
            .collect();
        let texture_cost = |key: &(usize, u32)| {
            let t = &sources[key.0].textures[key.1 as usize - 1];
            (t.rgba.len(), dxt_bytes(t.width, t.height, alpha_albedo.contains(key)))
        };
        let (raw, dxt) = textures_used.iter().map(texture_cost).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        println!(
            "- render: {} triangles, {} vertices ({:.1} MiB at 28 bytes), {} textures used: {:.1} MiB raw, {:.1} MiB as DXT ≤256² with mips",
            render_tris,
            render_verts,
            mib(render_verts * 28),
            textures_used.len(),
            mib(raw),
            mib(dxt)
        );
        let mut tri_counts: Vec<usize> = tiles.values().map(|t| t.0).collect();
        let mut tex_costs: Vec<usize> = tiles.values().map(|t| t.1.iter().map(|k| texture_cost(k).1).sum()).collect();
        println!(
            "- 64 m tiles: {} occupied; triangles per tile median {}, p95 {}, max {}; distinct DXT texture bytes per tile median {:.2} MiB, p95 {:.2} MiB, max {:.2} MiB",
            tiles.len(),
            percentile(&mut tri_counts.clone(), 0.5),
            percentile(&mut tri_counts, 0.95),
            tri_counts.iter().max().copied().unwrap_or(0),
            mib(percentile(&mut tex_costs.clone(), 0.5)),
            mib(percentile(&mut tex_costs.clone(), 0.95)),
            mib(tex_costs.iter().max().copied().unwrap_or(0))
        );
        // A 200 m radius around the densest tile centre: what a draw distance of 200 m holds.
        if let Some((&(tx, tz), _)) = tiles.iter().max_by_key(|(_, t)| t.0) {
            let near: Vec<&(usize, BTreeSet<(usize, u32)>)> = tiles
                .iter()
                .filter(|((x, z), _)| (((x - tx) as f32 * TILE).powi(2) + ((z - tz) as f32 * TILE).powi(2)).sqrt() <= 200.0)
                .map(|(_, t)| t)
                .collect();
            let near_tris: usize = near.iter().map(|t| t.0).sum();
            let near_tex: BTreeSet<&(usize, u32)> = near.iter().flat_map(|t| t.1.iter()).collect();
            let near_dxt: usize = near_tex.iter().map(|k| texture_cost(k).1).sum();
            println!(
                "- densest 200 m radius (around tile {tx},{tz}): {} tiles, {} triangles, {} textures, {:.1} MiB DXT",
                near.len(),
                near_tris,
                near_tex.len(),
                mib(near_dxt)
            );
        }

        // The simulation's heap for this collision: the stripped sim map into a real session.
        let before_load = CURRENT.load(Ordering::Relaxed);
        drop(sources);
        drop(props);
        let sim = map::strip_render(source);
        PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
        let base = CURRENT.load(Ordering::Relaxed);
        let started = std::time::Instant::now();
        match Session::load_map(&assets, "normal", Some(sim)) {
            Ok(session) => {
                let held = CURRENT.load(Ordering::Relaxed);
                println!(
                    "- simulation session: loaded in {:.1} s; heap {:.1} MiB beyond the stripped map ({:.1} MiB), peak during load {:.1} MiB",
                    started.elapsed().as_secs_f32(),
                    mib(held.saturating_sub(base)),
                    mib(base.saturating_sub(before_load)),
                    mib(PEAK.load(Ordering::Relaxed).saturating_sub(base))
                );
                drop(session);
            }
            Err(e) => println!("- simulation session: failed to load: {e}"),
        }
        println!();
    }
    Ok(())
}

/// The heap the game needs to load a district pack with streamed collision: the
/// pack bytes, the decoded sim map, and a session with a window, as `packs::load_park` and
/// `Session::load_map` do on the device.
pub fn window_heap(assets: &Path, pack: &Path) -> Result<(), String> {
    skate_sim::collision_window::configure(200.0, 60.0);
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let bytes = std::fs::read(pack).map_err(|e| format!("{}: {e}", pack.display()))?;
    let after_read = CURRENT.load(Ordering::Relaxed);
    let map = vita_pack::map::Map::from_pack(&bytes)?;
    let after_decode = CURRENT.load(Ordering::Relaxed);
    let peak_decode = PEAK.load(Ordering::Relaxed);
    let sim = map.sim;
    drop(bytes);
    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
    let base = CURRENT.load(Ordering::Relaxed);
    let session = Session::load_map(assets, "normal", Some(sim))?;
    let held = CURRENT.load(Ordering::Relaxed);
    println!(
        "{}: pack read {:.1} MiB; decoded map {:.1} MiB (peak during decode {:.1}); session load: peak {:.1} MiB above the map, session holds {:.1} MiB (window {:?})",
        pack.display(),
        mib(after_read - before),
        mib(after_decode - after_read),
        mib(peak_decode - before),
        mib(PEAK.load(Ordering::Relaxed).saturating_sub(base)),
        mib(held.saturating_sub(before)),
        session.window_stats()
    );
    Ok(())
}
