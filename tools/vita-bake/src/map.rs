//! `map_<name>.pak` (small retail parks, whole): collision and rails
//! unchanged (the `.skate` sim fields, see `vita_pack::map`); render geometry grouped
//! per material with albedo and lightmap textures at most 256², DXT-compressed.
//!
//! Spawn: the converted parks' authored spawn is checked with the real simulation (600
//! idle ticks: no teleport, still standing). Some conversions miss the structure the
//! spawn stands on (the converter places no DMO props), so when it fails the bake uses
//! the park's teleport destination, else the nearest validated upward floor triangle.
use std::path::Path;

use skate_data::retail_collision::visit_clusters;
use skate_data::skate_map::SkateMap;
use skate_sim::live::Session;
use vita_pack::map::{self, Draw, Map, Render};
use vita_pack::texture::{Format, Texture};

use crate::textures;

/// The parks (the three districts need a crop, see [`DISTRICTS`]).
pub const PARKS: [&str; 7] = [
    "StartPark",
    "MegaPark",
    "BlackBoxPark",
    "IndustrialSkatePark",
    "DownTownSkatePark",
    "MaloofMoneyCup",
    "SkateSchool",
];
/// Fixed exposure for the retail tone curve (the reference adapts between 0.75 and 2.5
/// towards a 0.25 target luminance).
const EXPOSURE: f32 = 1.4;
const SPAWN_TICKS: usize = 600;

pub fn bake(install: &Path, out: &Path, names: &[String]) -> Result<(), String> {
    let assets = install.join("assets");
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut total = 0;
    // The chosen spawns, for scripts/verify-maps.sh (ORACLE_SPAWN on the .skate runs).
    let mut spawns = String::new();
    for name in names {
        let started = std::time::Instant::now();
        let source = SkateMap::load(&install.join("maps").join(format!("{name}.skate")))?;
        let props_path = assets.join("private/native-props").join(format!("{name}.skate"));
        let props = if props_path.is_file() && !movable_props() {
            Some(SkateMap::parse_render_only(&std::fs::read(&props_path).map_err(|e| e.to_string())?)?)
        } else {
            None
        };
        let render = build_render(&source, props.as_ref())?;
        let (authored_spawn, authored_heading) = (source.spawn, source.heading);
        let mut sim = map::strip_render(source);
        let (spawn, heading, how) = choose_spawn(&assets, &sim)?;
        sim.spawn = spawn;
        sim.heading = heading;
        let pack = Map { sim, authored_spawn, authored_heading, render }.to_pack();
        let check = Map::from_pack(&pack)?;
        if check.sim.spawn != spawn || check.render.draws.is_empty() {
            return Err(format!("{name}: map pack does not read back"));
        }
        let path = out.join(format!("map_{name}.pak"));
        std::fs::write(&path, &pack).map_err(|e| e.to_string())?;
        total += pack.len();
        let r = &check.render;
        let line = format!("{name} {:?},{:?},{:?},{:?}", spawn[0], spawn[1], spawn[2], heading);
        println!("  spawn for ORACLE_SPAWN: {line}");
        spawns += &line;
        spawns.push('\n');
        let triangles: u32 = r.draws.iter().map(|d| d.index_count / 3).sum();
        println!(
            "{}: {} KiB; {} draws, {} triangles, {} vertices, {} textures ({} KiB); collision {} KiB as stored; spawn {} ({:.2}, {:.2}, {:.2}) [{:.1} s]",
            path.display(),
            pack.len() / 1024,
            r.draws.len(),
            triangles,
            r.vertex_data.len() / map::STRIDE,
            r.textures.len(),
            r.textures.iter().map(Texture::bytes).sum::<usize>() / 1024,
            check.sim.extensions.iter().map(|e| e.payload.len()).sum::<usize>() / 1024,
            how,
            spawn[0],
            spawn[1],
            spawn[2],
            started.elapsed().as_secs_f32()
        );
    }
    println!("maps total: {} KiB", total / 1024);
    std::fs::write(out.join("spawns.txt"), spawns).map_err(|e| e.to_string())?;
    Ok(())
}

/// `VITA_BAKE_MOVABLE_PROPS=1`: the props are drawn from `props.pak` (`vita-bake props`) at
/// their physics poses, so the park and district renders leave them out.
pub fn movable_props() -> bool {
    std::env::var("VITA_BAKE_MOVABLE_PROPS").is_ok_and(|v| v == "1")
}

/// The districts.
pub const DISTRICTS: [&str; 3] = ["Industrial", "University", "DownTown"];

/// `map_<District>.pak` with the simulation part only (streamed collision on the
/// device without the district render): collision, rails, materials, a validated spawn and
/// an empty render.
pub fn bake_district_sim(install: &Path, out: &Path, names: &[String]) -> Result<(), String> {
    let assets = install.join("assets");
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for name in names {
        let started = std::time::Instant::now();
        let source = SkateMap::load(&install.join("maps").join(format!("{name}.skate")))?;
        let horizon = [0, 1, 2].map(|i| source.environment.get(3 + i).copied().unwrap_or(0.4));
        let (authored_spawn, authored_heading) = (source.spawn, source.heading);
        let mut sim = map::strip_render(source);
        let (spawn, heading, how) = choose_spawn(&assets, &sim)?;
        sim.spawn = spawn;
        sim.heading = heading;
        let render = Render { vertex_data: Vec::new(), indices: Vec::new(), draws: Vec::new(), textures: Vec::new(), horizon, exposure: EXPOSURE };
        slim_for_sim(&mut sim)?;
        let defs: usize = sim.materials.iter().filter_map(|m| m.retail_definition.as_ref()).map(Vec::len).sum();
        let ext: Vec<String> = sim.extensions.iter().map(|e| format!("{}={:.1}MiB", String::from_utf8_lossy(&e.tag), e.payload.len() as f64 / 1048576.0)).collect();
        println!("  {} materials ({:.1} MiB of retail definitions), {} rails, extensions {}", sim.materials.len(), defs as f64 / 1048576.0, sim.rails.len(), ext.join(" "));
        let pack = Map { sim, authored_spawn, authored_heading, render }.to_pack();
        let check = Map::from_pack(&pack)?;
        if check.sim.spawn != spawn {
            return Err(format!("{name}: map pack does not read back"));
        }
        let path = out.join(format!("map_{name}.pak"));
        std::fs::write(&path, &pack).map_err(|e| e.to_string())?;
        println!("{}: {:.1} MiB (simulation only); spawn {how} ({:.1}, {:.1}, {:.1}) [{:.1} s]", path.display(), pack.len() as f64 / 1048576.0, spawn[0], spawn[1], spawn[2], started.elapsed().as_secs_f32());
    }
    Ok(())
}

/// The simulation reads only a few keys of the WMET manifest (29.6 MiB of JSON for University: the
/// grind provider's coordinate policy and, per rail, its source identity) and never the retail
/// material definitions (render only): both are cut down for the sim pack.
fn slim_for_sim(sim: &mut SkateMap) -> Result<(), String> {
        for extension in sim.extensions.iter_mut().filter(|e| &e.tag == b"WMET") {
            let manifest: serde_json::Value = serde_json::from_slice(&extension.payload).map_err(|e| format!("WMET: {e}"))?;
            let splines = manifest["grind_splines"].as_array().ok_or("WMET grind_splines array missing")?.clone();
            let trimmed = serde_json::json!({ "grind_coordinate_policy": manifest["grind_coordinate_policy"], "grind_splines": splines });
            extension.payload = serde_json::to_vec(&trimmed).map_err(|e| e.to_string())?;
        }
        for material in &mut sim.materials {
            material.retail_definition = None;
        }
    Ok(())
}

/// `district_<District>.pak`: the sim part without the collision archive,
/// the RWCM clusters as separate blobs with their boxes (windows read them by offset), the whole
/// world's broadphase limits and an empty render tile table (filled by the render tiles).
pub fn bake_district(install: &Path, out: &Path, names: &[String]) -> Result<(), String> {
    use vita_pack::district::DistrictBuilder;
    let assets = install.join("assets");
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    // The chosen spawns, for scripts/verify-windows.sh (ORACLE_SPAWN on the .skate runs).
    let mut spawns = String::new();
    for name in names {
        let started = std::time::Instant::now();
        let source = SkateMap::load(&install.join("maps").join(format!("{name}.skate")))?;
        let props_path = assets.join("private/native-props").join(format!("{name}.skate"));
        let props = if props_path.is_file() && !movable_props() {
            Some(SkateMap::parse_render_only(&std::fs::read(&props_path).map_err(|e| e.to_string())?)?)
        } else {
            None
        };
        let baked = crate::tiles::bake(&source, props.as_ref())?;
        crate::tiles::report(name, &baked);
        drop(props);
        let (authored_spawn, authored_heading) = (source.spawn, source.heading);
        let mut sim = map::strip_render(source);
        slim_for_sim(&mut sim)?;
        let (spawn, heading, how) = choose_spawn(&assets, &sim)?;
        sim.spawn = spawn;
        sim.heading = heading;
        spawns += &format!("{name} {:?},{:?},{:?},{:?}\n", spawn[0], spawn[1], spawn[2], heading);
        let at = sim.extensions.iter().position(|e| &e.tag == b"RWCM").ok_or("the district has no RWCM collision")?;
        let rwcm = sim.extensions.remove(at);
        let index = skate_data::retail_collision::index_archive(&rwcm.payload)?;
        let limits = skate_sim::collision_window::broadphase_limits(index.widest)?;
        let mut clusters = Vec::with_capacity(index.clusters.len());
        for (loc, info) in &index.clusters {
            clusters.push((*info, loc.bytes(&rwcm.payload)?.to_vec()));
        }
        let builder = DistrictBuilder { sim, authored_spawn, authored_heading, limits, clusters, tile_size: crate::tiles::TILE, tiles: baked.tiles, textures: baked.textures };
        let pack = builder.to_pack();
        let path = out.join(format!("district_{name}.pak"));
        std::fs::write(&path, &pack).map_err(|e| e.to_string())?;
        // Reads back through the ranged reader, and every cluster equals the archive's bytes.
        let (district, file) = vita_pack::district::open(&path)?;
        let ids: Vec<usize> = (0..district.clusters.len()).step_by(37).collect();
        for (id, bytes) in ids.iter().zip(skate_data::retail_collision::ClusterSource::read(&file, &ids)?) {
            if bytes != index.clusters[*id].0.bytes(&rwcm.payload)? {
                return Err(format!("{name}: cluster {id} reads back differently"));
            }
        }
        println!(
            "{}: {:.1} MiB; {} clusters ({:.1} MiB of collision), spawn {how} ({:.1}, {:.1}, {:.1}); limits {:?} [{:.1} s]",
            path.display(),
            pack.len() as f64 / 1048576.0,
            district.clusters.len(),
            rwcm.payload.len() as f64 / 1048576.0,
            spawn[0],
            spawn[1],
            spawn[2],
            limits,
            started.elapsed().as_secs_f32()
        );
    }
    std::fs::write(out.join("district_spawns.txt"), spawns).map_err(|e| e.to_string())?;
    Ok(())
}

/// A copy of the sim map (the types are not `Clone`), through the pack's encoding.
fn copy(sim: &SkateMap) -> Result<SkateMap, String> {
    map::decode_sim(&map::encode_sim(sim))
}

/// Whether the simulation keeps the skater standing at this spawn: 600 idle ticks with
/// no teleport, ending on the ground no lower than 1 m under the spawn.
fn spawn_holds(assets: &Path, sim: &SkateMap, spawn: [f32; 3], heading: f32) -> Result<bool, String> {
    let mut map = copy(sim)?;
    map.spawn = spawn;
    map.heading = heading;
    let mut session = Session::load_map(assets, "normal", Some(map))?;
    let idle = || skate_core::input::xbox::XboxState { buttons: 0, triggers: [0, 0], left: [0, 0], right: [0, 0] };
    let trace = std::env::var_os("VITA_BAKE_TRACE").is_some();
    for tick in 0..SPAWN_TICKS {
        let report = session.step(idle());
        if trace && tick < 12 {
            let (deck, speed) = session.board();
            eprintln!("    tick {tick} state {} deck {deck:?} speed {speed:.2}", session.state_name());
        }
        if report.error.is_some() || session.state_name() == "Teleporting" {
            eprintln!("  spawn {spawn:?} rejected at tick {tick}: {}", report.error.as_deref().unwrap_or("teleported"));
            return Ok(false);
        }
    }
    let (deck, _) = session.board();
    let holds = session.state_name() == "PhysicsGround" && deck[1] > spawn[1] - 1.0;
    if !holds {
        eprintln!("  spawn {spawn:?} rejected: state {} deck height {:.2}", session.state_name(), deck[1]);
    }
    Ok(holds)
}

fn choose_spawn(assets: &Path, sim: &SkateMap) -> Result<([f32; 3], f32, &'static str), String> {
    if spawn_holds(assets, sim, sim.spawn, sim.heading)? {
        return Ok((sim.spawn, sim.heading, "authored"));
    }
    for (position, heading) in teleports(assets, &sim.name)? {
        if spawn_holds(assets, sim, position, heading)? {
            return Ok((position, heading, "teleport destination"));
        }
    }
    for position in floor_candidates(sim)?.into_iter().take(96) {
        if spawn_holds(assets, sim, position, sim.heading)? {
            return Ok((position, sim.heading, "nearest validated floor"));
        }
    }
    Err(format!("{}: no spawn keeps the skater standing", sim.name))
}

/// The park's authored teleport destinations (`private/teleports.json`).
fn teleports(assets: &Path, name: &str) -> Result<Vec<([f32; 3], f32)>, String> {
    let text = std::fs::read(assets.join("private/teleports.json")).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for d in v["destinations"].as_array().into_iter().flatten() {
        if d["map"].as_str() != Some(name) {
            continue;
        }
        let Some(m) = d["matrix"].as_array() else { continue };
        let row = |i: usize| -> Option<[f32; 4]> {
            let r = m.get(i)?.as_array()?;
            Some([0, 1, 2, 3].map(|j| r.get(j).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32))
        };
        let (Some(forward), Some(position)) = (row(2), row(3)) else { continue };
        out.push(([position[0], position[1], position[2]], forward[0].atan2(forward[2])));
    }
    Ok(out)
}

/// Whether a triangle is a large, level, upward floor under (or near) the spawn.
fn floor_centroid(points: [[f32; 3]; 3], spawn: [f32; 3], max_above: f32) -> Option<(f32, [f32; 3])> {
    let [a, b, c] = points;
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    // Large and level, facing up (the collision's winding gives the face).
    if len < 2.0 || n[1] / len < 0.98 {
        return None;
    }
    let p = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
    if p[1] > spawn[1] + max_above {
        return None;
    }
    Some((((p[0] - spawn[0]).powi(2) + (p[2] - spawn[2]).powi(2)).sqrt(), p))
}

/// Centroids of upward-facing collision triangles near the authored spawn, nearest
/// first (horizontal distance). Prefers retail `RWCM`; custom maps use portable
/// triangle collision instead.
fn floor_candidates(sim: &SkateMap) -> Result<Vec<[f32; 3]>, String> {
    let spawn = sim.spawn;
    let mut out: Vec<(f32, [f32; 3])> = Vec::new();
    if let Some(rwcm) = sim.extensions.iter().find(|e| &e.tag == b"RWCM") {
        visit_clusters(&rwcm.payload, |_, triangles| {
            for t in triangles {
                // Special surfaces (TU3 82DB8120, b3c9679): type 6 sends the skater back
                // (out of bounds), types 9 and 12 (water) make him bail.
                if t.has_surface && matches!((t.surface >> 7) & 31, 6 | 9 | 12) {
                    continue;
                }
                if let Some(hit) = floor_centroid(t.points, spawn, 0.5) {
                    out.push(hit);
                }
            }
            Ok(())
        })?;
    } else if !sim.geometry.collision.is_empty() {
        // Custom .skate maps: no RWCM. Authored spawn is often (0,0,0), so allow
        // floors well above it; spawn_holds still validates each candidate.
        for tri in &sim.geometry.collision {
            if let Some(hit) = floor_centroid(tri.points, spawn, 500.0) {
                out.push(hit);
            }
        }
    } else {
        return Err("no retail or portable collision".into());
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(out.into_iter().map(|(_, p)| p).collect())
}

/// Material family from a retail definition (skate-game `retail_render::Definition`):
/// 16 bytes, shader name (u32 length + text), family u32.
pub(crate) fn family(definition: &[u8]) -> Option<(String, u32)> {
    let n = u32::from_le_bytes(definition.get(16..20)?.try_into().ok()?) as usize;
    let shader = String::from_utf8(definition.get(20..20 + n)?.to_vec()).ok()?;
    let stored = u32::from_le_bytes(definition.get(20 + n..24 + n)?.try_into().ok()?);
    let family = if matches!(shader.as_str(), "water.default" | "water.alpha" | "water.skatepark") { 33 } else { stored };
    Some((shader, family))
}

struct Group {
    source: usize,
    albedo: u32,
    lightmap: u32,
    cutoff: Option<f32>,
    color: [f32; 4],
    triangles: Vec<[u32; 3]>,
}

/// Material draw groups of the map and its render-only props package.
fn build_render(map: &SkateMap, props: Option<&SkateMap>) -> Result<Render, String> {
    let sources: Vec<&SkateMap> = std::iter::once(map).chain(props).collect();
    let mut builder = RenderBuilder::default();
    let triangles: Vec<(usize, [u32; 3])> = sources
        .iter()
        .enumerate()
        .flat_map(|(s, src)| src.geometry.indices.chunks_exact(3).map(move |t| (s, [t[0], t[1], t[2]])))
        .collect();
    builder.add(&sources, &triangles)?;
    // Opaque first, alpha-tested last.
    builder.draws.sort_by_key(|d| d.alpha_cutoff.is_some());
    let e = &map.environment;
    Ok(Render {
        vertex_data: builder.vertex_data,
        indices: builder.indices,
        draws: builder.draws,
        textures: builder.textures,
        horizon: [e.get(3).copied().unwrap_or(0.3), e.get(4).copied().unwrap_or(0.4), e.get(5).copied().unwrap_or(0.5)],
        exposure: EXPOSURE,
    })
}

/// The movable props' render: one run of draws per template range (`(first_index,
/// index_count)` of the MOBJ records, template space), sharing vertices and textures; draws of
/// a template are opaque first. Returns the render and each template's draw range, in the order
/// of `templates`.
pub fn build_prop_render(props: &SkateMap, templates: &[(u32, u32)]) -> Result<(Render, Vec<std::ops::Range<usize>>), String> {
    let sources = [props];
    let mut builder = RenderBuilder::default();
    let mut ranges = Vec::with_capacity(templates.len());
    for &(first, count) in templates {
        let range = first as usize..(first + count) as usize;
        let triangles: Vec<(usize, [u32; 3])> = props.geometry.indices[range].chunks_exact(3).map(|t| (0, [t[0], t[1], t[2]])).collect();
        let start = builder.draws.len();
        builder.add(&sources, &triangles)?;
        builder.draws[start..].sort_by_key(|d| d.alpha_cutoff.is_some());
        ranges.push(start..builder.draws.len());
    }
    Ok((
        Render { vertex_data: builder.vertex_data, indices: builder.indices, draws: builder.draws, textures: builder.textures, horizon: [0.0; 3], exposure: EXPOSURE },
        ranges,
    ))
}

/// Builds draws from triangles of one or more sources into shared vertex, index and texture
/// arrays: grouped by material (albedo, lightmap, alpha cutoff, colour), each texture baked once,
/// re-indexed in chunks of at most 65,536 vertices.
#[derive(Default)]
struct RenderBuilder {
    vertex_data: Vec<u8>,
    indices: Vec<u16>,
    draws: Vec<Draw>,
    textures: Vec<Texture>,
    texture_keys: Vec<(usize, u32, bool)>,
}

impl RenderBuilder {
    fn texture(&mut self, sources: &[&SkateMap], source: usize, index: u32, alpha: bool) -> Result<Option<u16>, String> {
        if index == 0 {
            return Ok(None);
        }
        if let Some(i) = self.texture_keys.iter().position(|k| *k == (source, index, alpha)) {
            return Ok(Some(i as u16));
        }
        let t = &sources[source].textures[index as usize - 1];
        let format = if alpha { Format::Dxt5 } else { Format::Dxt1 };
        self.textures.push(textures::bake(&t.rgba, t.width, t.height, format)?);
        self.texture_keys.push((source, index, alpha));
        Ok(Some((self.textures.len() - 1) as u16))
    }

    fn add(&mut self, sources: &[&SkateMap], triangles: &[(usize, [u32; 3])]) -> Result<(), String> {
        let mut groups: Vec<Group> = Vec::new();
        let mut skipped = 0usize;
        for &(s, tri) in triangles {
            let src = sources[s];
            let material = &src.materials[src.geometry.vertices[tri[0] as usize].material as usize - 1];
            // No water (water shaders are dropped).
            if material.retail_definition.as_deref().and_then(family).is_some_and(|(_, f)| f == 14 || (30..=33).contains(&f)) {
                skipped += 1;
                continue;
            }
            let cutoff = match material.alpha_mode {
                0 => None,
                1 => Some(material.alpha_cutoff),
                _ => Some(0.5),
            };
            let color = [material.color[0], material.color[1], material.color[2], 1.0];
            let key = (s, material.textures[0], material.textures[1], cutoff.map(f32::to_bits), color.map(f32::to_bits));
            let group = match groups.iter().position(|g| (g.source, g.albedo, g.lightmap, g.cutoff.map(f32::to_bits), g.color.map(f32::to_bits)) == key) {
                Some(i) => i,
                None => {
                    groups.push(Group { source: s, albedo: material.textures[0], lightmap: material.textures[1], cutoff, color, triangles: Vec::new() });
                    groups.len() - 1
                }
            };
            groups[group].triangles.push(tri);
        }
        if skipped > 0 {
            println!("  {skipped} water triangles not rendered");
        }
        for g in &groups {
            let src = sources[g.source];
            let albedo = self.texture(sources, g.source, g.albedo, g.cutoff.is_some())?;
            let lightmap = self.texture(sources, g.source, g.lightmap, false)?;
            // Re-index into chunks of at most 65,536 vertices.
            let mut chunk_start = 0;
            while chunk_start < g.triangles.len() {
                let first_vertex = (self.vertex_data.len() / map::STRIDE) as u32;
                let first_index = self.indices.len() as u32;
                let mut remap = std::collections::HashMap::new();
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                let mut end = chunk_start;
                while end < g.triangles.len() {
                    let tri = g.triangles[end];
                    let new = tri.iter().filter(|v| !remap.contains_key(*v)).count();
                    if remap.len() + new > 65_536 {
                        break;
                    }
                    for v in tri {
                        let next = remap.len() as u16;
                        let vertex_data = &mut self.vertex_data;
                        let local = *remap.entry(v).or_insert_with(|| {
                            let vx = &src.geometry.vertices[v as usize];
                            for c in vx.position.iter().chain(&vx.uv).chain(&vx.lightmap_uv) {
                                vertex_data.extend_from_slice(&c.to_le_bytes());
                            }
                            for i in 0..3 {
                                lo[i] = lo[i].min(vx.position[i]);
                                hi[i] = hi[i].max(vx.position[i]);
                            }
                            next
                        });
                        self.indices.push(local);
                    }
                    end += 1;
                }
                self.draws.push(Draw {
                    first_vertex,
                    vertex_count: remap.len() as u32,
                    first_index,
                    index_count: self.indices.len() as u32 - first_index,
                    albedo,
                    lightmap,
                    color: g.color,
                    alpha_cutoff: g.cutoff,
                    bounds_min: lo,
                    bounds_max: hi,
                });
                chunk_start = end;
            }
        }
        Ok(())
    }
}

/// Survey of the movable props of each world (`native-props/<name>.skate`): MOBJ schema,
/// instances, distinct template ranges, triangles, textures.
pub fn prop_stats(install: &Path, names: &[&str]) -> Result<(), String> {
    for name in names {
        let path = install.join("assets/private/native-props").join(format!("{name}.skate"));
        let Ok(bytes) = std::fs::read(&path) else {
            println!("{name}: no props");
            continue;
        };
        let map = SkateMap::parse_render_only(&bytes)?;
        for extension in map.extensions.iter().filter(|e| e.tag == *b"MOBJ") {
            let objects = skate_data::skate_map::parse_static_objects(&map, extension)?;
            let mut templates: Vec<(u32, u32)> = objects.iter().map(|o| (o.first_index, o.index_count)).collect();
            templates.sort_unstable();
            templates.dedup();
            let template_triangles: u32 = templates.iter().map(|t| t.1 / 3).sum();
            let instance_triangles: u32 = objects.iter().map(|o| o.index_count / 3).sum();
            let dynamic = objects.iter().filter(|o| o.physics.density > 0.0).count();
            println!(
                "{name}: schema {} {} objects ({} with density), {} template ranges, {} template triangles, {} drawn triangles; geometry {} triangles; {} textures {:.1} MB RGBA",
                extension.schema,
                objects.len(),
                dynamic,
                templates.len(),
                template_triangles,
                instance_triangles,
                map.geometry.indices.len() / 3,
                map.textures.len(),
                map.textures.iter().map(|t| t.rgba.len()).sum::<usize>() as f64 / 1e6
            );
        }
    }
    Ok(())
}

/// `props.pak`, the movable props of every world that has them (from an install whose
/// setup wrote MOBJ schema 4 placements): per world `private/native-props/<name>.props` (the
/// simulation's package, `skate_data::prop_package`), `<name>.json` (types, grab splines, grind
/// rails) and `<name>.render` (`vita_pack::map::PropRender`).
pub fn bake_props(install: &Path, out: &Path) -> Result<(), String> {
    use vita_pack::files::File;
    let dir = install.join("assets/private/native-props");
    let mut files = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".skate").map(str::to_owned))
        .collect();
    names.sort();
    for name in names {
        let props = SkateMap::parse_render_only(&std::fs::read(dir.join(format!("{name}.skate"))).map_err(|e| e.to_string())?)?;
        let mut objects = Vec::new();
        for extension in props.extensions.iter().filter(|e| e.tag == *b"MOBJ") {
            if extension.schema != 4 {
                return Err(format!("{name}: MOBJ schema {} (rerun setup: movable props need schema 4)", extension.schema));
            }
            objects.extend(skate_data::skate_map::parse_static_objects(&props, extension)?);
        }
        if objects.is_empty() {
            println!("  {name}: no prop placements, skipped");
            continue;
        }
        let mut templates: Vec<(u32, u32)> = objects.iter().map(|o| (o.first_index, o.index_count)).collect();
        templates.sort_unstable();
        templates.dedup();
        let (render, ranges) = build_prop_render(&props, &templates)?;
        let prop_render = vita_pack::map::PropRender {
            templates: templates
                .iter()
                .zip(&ranges)
                .map(|(&(first_index, index_count), r)| vita_pack::map::PropTemplate { first_index, index_count, draws: [r.start as u32, r.end as u32] })
                .collect(),
            render,
        };
        let render_bytes = prop_render.to_pack();
        if vita_pack::map::PropRender::from_pack(&render_bytes)? != prop_render {
            return Err(format!("{name}: props render does not read back"));
        }
        let package = skate_data::prop_package::encode(&props);
        let back = skate_data::prop_package::decode(&package)?;
        if back.geometry.indices != props.geometry.indices || back.extensions != props.extensions || back.materials != props.materials {
            return Err(format!("{name}: props package does not read back"));
        }
        let r = &prop_render.render;
        println!(
            "  {name}: {} props, {} templates, {} draws, {} triangles, {} textures ({} KiB); package {} KiB",
            objects.len(),
            templates.len(),
            r.draws.len(),
            r.draws.iter().map(|d| d.index_count / 3).sum::<u32>(),
            r.textures.len(),
            r.textures.iter().map(Texture::bytes).sum::<usize>() / 1024,
            package.len() / 1024
        );
        let base = format!("private/native-props/{name}");
        files.push(File { path: format!("{base}.props"), bytes: package });
        files.push(File { path: format!("{base}.render"), bytes: render_bytes });
        // The sidecar, with what the simulation reads (the type data, grab and grind splines, collision shapes);
        // its instances, hotpoints and ped plugins come with the pedestrians.
        let json = dir.join(format!("{name}.json"));
        if json.is_file() {
            let full: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).map_err(|e| e.to_string())?).map_err(|e| format!("{name}.json: {e}"))?;
            // Since upstream 618e824: `collision`, the templates' retail collision shapes.
            let kept: serde_json::Map<String, serde_json::Value> = ["map", "simulation", "types", "grab_splines", "grind_splines", "collision"]
                .iter()
                .filter_map(|k| Some(((*k).to_owned(), full.get(*k)?.clone())))
                .collect();
            files.push(File { path: format!("{base}.json"), bytes: serde_json::to_vec(&serde_json::Value::Object(kept)).map_err(|e| e.to_string())? });
        }
    }
    // The census values of the props' streaming (`living_world::dmo_stream::read_data`): the
    // `dynamicobjects` records of the living world's tables, under their own paths.
    let tables_path = install.join("assets/private/living_world/tables.json");
    let tables: serde_json::Value = serde_json::from_slice(&std::fs::read(&tables_path).map_err(|e| format!("{}: {e}", tables_path.display()))?).map_err(|e| e.to_string())?;
    let pick = |class: &str| tables.pointer(&format!("/classes/{class}/dynamicobjects")).cloned().ok_or(format!("tables.json: no {class}/dynamicobjects"));
    let slim = serde_json::json!({ "classes": {
        "livingworld_census_ranges": { "dynamicobjects": pick("livingworld_census_ranges")? },
        "livingworld": { "dynamicobjects": pick("livingworld")? },
    }});
    files.push(File { path: "private/living_world/tables.json".into(), bytes: serde_json::to_vec(&slim).map_err(|e| e.to_string())? });
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let pack = vita_pack::files::to_pack(&files);
    std::fs::write(out.join("props.pak"), &pack).map_err(|e| e.to_string())?;
    // The index next to it (as the audio packs): the Vita reads it instead of one pass over the pack.
    let index = vita_pack::files::index_of(&pack)?;
    std::fs::write(out.join("props.pak.idx"), vita_pack::files::encode_index(&pack[24..56], &index)).map_err(|e| e.to_string())?;
    println!("{}: {} files, {} KiB", out.join("props.pak").display(), files.len(), pack.len() / 1024);
    Ok(())
}
