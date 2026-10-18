//! Render tiles of a district: the converted `.skate` render geometry (the map and
//! its render-only props package) cut into 64 m tiles, each with three levels of detail that are
//! simplified from the original (`meshopt`, borders locked so the pieces still meet), merged per
//! material, and the DXT textures they share (at most 256², mipmapped), indexed by a table.
use std::collections::HashMap;

use skate_data::skate_map::SkateMap;
use vita_pack::district::{GREY_LIGHTMAP, NO_TEXTURE, TileDraw, TileMesh, encode_lod};
use vita_pack::map::STRIDE;
use vita_pack::texture::{Format, Texture};

use crate::textures;

pub const TILE: f32 = 64.0;
/// The most triangles a tile keeps per level of detail (a tile smaller than that is not reduced).
pub const CAPS: [usize; 3] = [10_000, 3_000, 800];
/// Simplification error limits per level, in metres (the target counts are met if they are reached
/// within it). Distant levels also drop tiny disconnected pieces.
const ERRORS: [f32; 3] = [0.2, 1.0, 4.0];
/// The far level draws without lightmaps, and in the others the small pieces do: pieces that differ
/// only by their lightmap merge into one draw (the draw count is what limits a district's frame
/// rate, not its triangles; 65% of the draws are of at most 64 triangles).
const IGNORE_LIGHTMAP: [bool; 3] = [false, false, true];
/// A piece of at most this many triangles loses its lightmap in levels 0 and 1.
const SMALL_PIECE: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    source: usize,
    albedo: u32,
    lightmap: u32,
    cutoff: Option<u32>,
    color: [u32; 4],
}

type Triangles = Vec<[u32; 3]>;

/// A vertex used by triangles of one piece (a tile and material) only, or by several.
const UNOWNED: u32 = u32::MAX;
const SHARED: u32 = u32::MAX - 1;

pub struct Baked {
    pub tiles: Vec<(i32, i32, [Vec<u8>; 3])>,
    pub textures: Vec<Texture>,
    pub stats: Stats,
}

#[derive(Default)]
pub struct Stats {
    pub source_triangles: usize,
    pub water_skipped: usize,
    /// Per level: triangles, draws and vertices over all tiles.
    pub lod_triangles: [usize; 3],
    pub lod_draws: [usize; 3],
    pub lod_vertices: [usize; 3],
    pub lod_bytes: [usize; 3],
    pub per_tile_triangles: [Vec<usize>; 3],
    pub per_tile_draws: [Vec<usize>; 3],
    /// Draws of at most 16 and of at most 64 triangles, per level.
    pub tiny_draws: [(usize, usize); 3],
}

fn position_key(p: [f32; 3]) -> [i32; 3] {
    p.map(|c| (c * 1000.0).round() as i32)
}

fn tile_of(p: f32) -> i32 {
    (p / TILE).floor() as i32
}

pub fn bake(map: &SkateMap, props: Option<&SkateMap>) -> Result<Baked, String> {
    let sources: Vec<&SkateMap> = std::iter::once(map).chain(props).collect();
    let mut stats = Stats::default();

    // 1. Triangles by tile and material.
    let mut tiles: HashMap<(i32, i32), HashMap<Key, Triangles>> = HashMap::new();
    // Per source, which piece owns each vertex: vertices shared by pieces (seams between tiles and
    // between materials) are locked when simplifying, so the pieces still meet.
    // The sharing is by position (to a millimetre): the retail meshes meet at coincident vertices
    // that are separate in the file, and moving one side of such a seam would open a gap.
    let mut owners: HashMap<[i32; 3], u32> = HashMap::new();
    let mut piece_ids: HashMap<((i32, i32), Key), u32> = HashMap::new();
    for (s, src) in sources.iter().enumerate() {
        for tri in src.geometry.indices.chunks_exact(3) {
            let v = [tri[0], tri[1], tri[2]].map(|i| &src.geometry.vertices[i as usize]);
            let material = &src.materials[v[0].material as usize - 1];
            if material.retail_definition.as_deref().and_then(crate::map::family).is_some_and(|(_, f)| f == 14 || (30..=33).contains(&f)) {
                stats.water_skipped += 1;
                continue;
            }
            stats.source_triangles += 1;
            let cutoff = match material.alpha_mode {
                0 => None,
                1 => Some(material.alpha_cutoff),
                _ => Some(0.5),
            };
            let color = [material.color[0], material.color[1], material.color[2], 1.0];
            let cx = (v[0].position[0] + v[1].position[0] + v[2].position[0]) / 3.0;
            let cz = (v[0].position[2] + v[1].position[2] + v[2].position[2]) / 3.0;
            let key = Key { source: s, albedo: material.textures[0], lightmap: material.textures[1], cutoff: cutoff.map(f32::to_bits), color: color.map(f32::to_bits) };
            let tile = (tile_of(cx), tile_of(cz));
            let next = piece_ids.len() as u32;
            let piece = *piece_ids.entry((tile, key)).or_insert(next);
            for v in v {
                let o = owners.entry(position_key(v.position)).or_insert(UNOWNED);
                if *o == UNOWNED {
                    *o = piece;
                } else if *o != piece {
                    *o = SHARED;
                }
            }
            tiles.entry(tile).or_default().entry(key).or_default().push([tri[0], tri[1], tri[2]]);
        }
    }

    // 2. The textures that are used, baked in parallel (alpha materials keep their alpha).
    let mut wanted: Vec<(usize, u32, bool)> = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        let mut keys: Vec<&Key> = tiles.values().flat_map(|t| t.keys()).collect();
        keys.sort_by_key(|k| (k.source, k.albedo, k.lightmap));
        for k in keys {
            for (index, alpha) in [(k.albedo, k.cutoff.is_some()), (k.lightmap, false)] {
                if index != 0 && seen.insert((k.source, index, alpha)) {
                    wanted.push((k.source, index, alpha));
                }
            }
        }
    }
    let baked: Vec<std::sync::Mutex<Option<Result<Texture, String>>>> = wanted.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(source, index, alpha)) = wanted.get(i) else { break };
                let t = &sources[source].textures[index as usize - 1];
                let format = if alpha { Format::Dxt5 } else { Format::Dxt1 };
                *baked[i].lock().unwrap() = Some(textures::bake(&t.rgba, t.width, t.height, format));
            });
        }
    });
    let mut textures_out = Vec::with_capacity(wanted.len());
    let mut texture_id: HashMap<(usize, u32, bool), u32> = HashMap::new();
    for (i, key) in wanted.iter().enumerate() {
        textures_out.push(baked[i].lock().unwrap().take().unwrap()?);
        texture_id.insert(*key, i as u32);
    }

    // 3. Each tile: three levels of detail.
    let mut ordered: Vec<((i32, i32), HashMap<Key, Triangles>)> = tiles.into_iter().collect();
    ordered.sort_by_key(|(k, _)| *k);
    let mut out = Vec::with_capacity(ordered.len());
    for ((cx, cz), groups) in ordered {
        let mut groups: Vec<(Key, Triangles)> = groups.into_iter().collect();
        // Opaque first, then alpha-tested, then by textures (a stable draw order).
        groups.sort_by_key(|(k, _)| (k.cutoff.is_some(), k.source, k.albedo, k.lightmap, k.color));
        let total: usize = groups.iter().map(|(_, t)| t.len()).sum();
        let mut blobs: [Vec<u8>; 3] = Default::default();
        for lod in 0..3 {
            let ratio = (CAPS[lod] as f32 / total as f32).min(1.0);
            let mut meshes: Vec<TileMesh> = Vec::new();
            let (mut tris, mut draws, mut verts) = (0, 0, 0);
            // (key, triangles, drawn with the grey lightmap).
            let mut level_groups: Vec<(Key, Triangles, bool)> = Vec::new();
            for (key, triangles) in &groups {
                let grey = IGNORE_LIGHTMAP[lod] || (triangles.len() <= SMALL_PIECE && key.lightmap != 0);
                if !grey {
                    level_groups.push((*key, triangles.clone(), false));
                    continue;
                }
                let k = Key { lightmap: 0, ..*key };
                match level_groups.iter_mut().find(|(o, _, g)| *o == k && *g) {
                    Some((_, t, _)) => t.extend_from_slice(triangles),
                    None => level_groups.push((k, triangles.clone(), key.lightmap != 0 || IGNORE_LIGHTMAP[lod])),
                }
            }
            for (key, triangles, grey) in &level_groups {
                let src = sources[key.source];
                let piece = Piece::new(src, triangles, &owners);
                let target = ((triangles.len() as f32 * ratio) as usize).max(triangles.len().min(8));
                let indices = if target < triangles.len() { piece.simplify(target, ERRORS[lod], lod > 0) } else { piece.indices.clone() };
                if indices.is_empty() {
                    continue;
                }
                let albedo = if key.albedo == 0 { NO_TEXTURE } else { texture_id[&(key.source, key.albedo, key.cutoff.is_some())] };
                let lightmap = if *grey { GREY_LIGHTMAP } else if key.lightmap == 0 { NO_TEXTURE } else { texture_id[&(key.source, key.lightmap, false)] };
                tris += indices.len() / 3;
                draws += 1;
                stats.tiny_draws[lod].0 += usize::from(indices.len() / 3 <= 16);
                stats.tiny_draws[lod].1 += usize::from(indices.len() / 3 <= 64);
                add_piece(&mut meshes, &piece, &indices, TileDraw {
                    first_index: 0,
                    index_count: 0,
                    albedo,
                    lightmap,
                    color: key.color.map(f32::from_bits),
                    alpha_cutoff: key.cutoff.map(f32::from_bits),
                    bounds_min: [0.0; 3],
                    bounds_max: [0.0; 3],
                });
            }
            verts += meshes.iter().map(|m| m.vertex_data.len() / STRIDE).sum::<usize>();
            blobs[lod] = encode_lod(&meshes);
            stats.lod_triangles[lod] += tris;
            stats.lod_draws[lod] += draws;
            stats.lod_vertices[lod] += verts;
            stats.lod_bytes[lod] += blobs[lod].len();
            stats.per_tile_triangles[lod].push(tris);
            stats.per_tile_draws[lod].push(draws);
        }
        out.push((cx, cz, blobs));
    }
    Ok(Baked { tiles: out, textures: textures_out, stats })
}

/// One material's triangles in one tile, with its own compact vertices.
struct Piece {
    /// position (3), uv (2), lightmap uv (2) per vertex, as f32.
    vertices: Vec<[f32; 7]>,
    indices: Vec<u32>,
    /// Vertices shared with other pieces: not moved by simplification.
    locked: Vec<bool>,
}

impl Piece {
    fn new(src: &SkateMap, triangles: &Triangles, owners: &HashMap<[i32; 3], u32>) -> Self {
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut vertices = Vec::new();
        let mut locked = Vec::new();
        let mut indices = Vec::with_capacity(triangles.len() * 3);
        for tri in triangles {
            for &v in tri {
                let local = *remap.entry(v).or_insert_with(|| {
                    let x = &src.geometry.vertices[v as usize];
                    vertices.push([x.position[0], x.position[1], x.position[2], x.uv[0], x.uv[1], x.lightmap_uv[0], x.lightmap_uv[1]]);
                    locked.push(owners.get(&position_key(x.position)) == Some(&SHARED));
                    (vertices.len() - 1) as u32
                });
                indices.push(local);
            }
        }
        Self { vertices, indices, locked }
    }

    fn simplify(&self, target_triangles: usize, error: f32, prune: bool) -> Vec<u32> {
        let bytes: Vec<u8> = self.vertices.iter().flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes())).collect();
        let adapter = meshopt::VertexDataAdapter::new(&bytes, 28, 0).expect("vertex layout");
        let attributes: Vec<f32> = self.vertices.iter().flat_map(|v| [v[3], v[4], v[5], v[6]]).collect();
        let options = meshopt::SimplifyOptions::ErrorAbsolute | if prune { meshopt::SimplifyOptions::Prune } else { meshopt::SimplifyOptions::None };
        meshopt::simplify_with_attributes_and_locks(
            &self.indices,
            &adapter,
            &attributes,
            &[0.5, 0.5, 1.0, 1.0],
            4 * 4,
            &self.locked,
            target_triangles * 3,
            error,
            options,
            None,
        )
    }
}

/// Appends a piece to the last mesh that has room (65,536 vertices), as one draw.
fn add_piece(meshes: &mut Vec<TileMesh>, piece: &Piece, indices: &[u32], mut draw: TileDraw) {
    // Only the vertices the simplified indices use.
    let mut remap: HashMap<u32, u16> = HashMap::new();
    let mut used: Vec<u32> = Vec::new();
    for &i in indices {
        remap.entry(i).or_insert_with(|| {
            used.push(i);
            (used.len() - 1) as u16
        });
        if used.len() > 65_535 {
            break;
        }
    }
    // A piece whose vertices do not fit one mesh is split by triangles.
    if used.len() > 65_535 {
        let half = indices.len() / 6 * 3;
        add_piece(meshes, piece, &indices[..half], draw.clone());
        add_piece(meshes, piece, &indices[half..], draw);
        return;
    }
    if meshes.last().is_none_or(|m| m.vertex_data.len() / STRIDE + used.len() > 65_536) {
        meshes.push(TileMesh::default());
    }
    let mesh = meshes.last_mut().unwrap();
    let base = (mesh.vertex_data.len() / STRIDE) as u32;
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for &v in &used {
        let x = &piece.vertices[v as usize];
        for c in x {
            mesh.vertex_data.extend_from_slice(&c.to_le_bytes());
        }
        for i in 0..3 {
            lo[i] = lo[i].min(x[i]);
            hi[i] = hi[i].max(x[i]);
        }
    }
    draw.first_index = mesh.indices.len() as u32;
    draw.index_count = indices.len() as u32;
    draw.bounds_min = lo;
    draw.bounds_max = hi;
    for &i in indices {
        mesh.indices.push((base + u32::from(remap[&i])) as u16);
    }
    mesh.draws.push(draw);
}

fn percentile(v: &[usize], p: f64) -> usize {
    let mut v = v.to_vec();
    v.sort_unstable();
    v.get(((v.len().max(1) - 1) as f64 * p) as usize).copied().unwrap_or(0)
}

pub fn report(name: &str, b: &Baked) {
    let s = &b.stats;
    println!(
        "{name}: {} source triangles ({} water skipped), {} tiles, {} textures ({:.1} MiB)",
        s.source_triangles,
        s.water_skipped,
        b.tiles.len(),
        b.textures.len(),
        b.textures.iter().map(Texture::bytes).sum::<usize>() as f64 / 1048576.0
    );
    for lod in 0..3 {
        println!(
            "  LOD{lod}: {} triangles, {} draws, {} vertices, {:.1} MiB; per tile triangles median {} p95 {} max {}, draws median {} p95 {} max {}",
            s.lod_triangles[lod],
            s.lod_draws[lod],
            s.lod_vertices[lod],
            s.lod_bytes[lod] as f64 / 1048576.0,
            percentile(&s.per_tile_triangles[lod], 0.5),
            percentile(&s.per_tile_triangles[lod], 0.95),
            s.per_tile_triangles[lod].iter().max().copied().unwrap_or(0),
            percentile(&s.per_tile_draws[lod], 0.5),
            percentile(&s.per_tile_draws[lod], 0.95),
            s.per_tile_draws[lod].iter().max().copied().unwrap_or(0),
        );
        println!("        draws of at most 16 triangles: {}, of at most 64: {} (of {})", s.tiny_draws[lod].0, s.tiny_draws[lod].1, s.lod_draws[lod]);
    }
}
