//! `district_<name>.pak` (kind `District`): one campaign district, read
//! by offset instead of whole.
//!
//! A district is too big to load (University: 24 MiB of collision, and the render will be far
//! more), so the file opens by reading only its **index**:
//! - the **sim** part of the map (spawn, materials, rails and grind metadata: everything the
//!   simulation reads except the collision archive), small and resident;
//! - the **clusters**: for each RWCM cluster its box, triangle count and decoding parameters,
//!   and where its bytes are (the collision windows read the clusters they need);
//! - the whole world's broadphase **limits** (so opening decodes nothing);
//! - the render **tiles**: a grid of tiles, each with up to three levels of detail
//!   at an offset (a [`TileMesh`] list: vertices, u16 indices and material draws), and the
//!   **textures** they share (a table of DXT textures, read one at a time).
//!
//! After the index come the blobs. Offsets are relative to the end of the index. The file is
//! never read whole: [`DistrictFile::read_range`] seeks and reads.
use crate::{Kind, Reader, Writer, HEADER_LEN, MAGIC};
use skate_data::retail_collision::{ClusterInfo, ClusterParams, ClusterSource};
use skate_data::skate_map::SkateMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

/// v1: clusters; v2: render tiles with levels of detail and the texture table.
pub const VERSION: u32 = 2;
/// Reads whose gap is at most this many bytes are merged into one.
const MERGE_GAP: u64 = 8 * 1024;
/// Textures closer than this in the file are read together (`DistrictFile::read_textures`).
const TEXTURE_GAP: u64 = 32 * 1024;
const TEXTURE_RUN: u64 = 512 * 1024;

/// One render tile: its cell and where its levels of detail are (offset, size; size 0 = none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub cx: i32,
    pub cz: i32,
    pub lods: [(u64, u32); 3],
}

/// Map vertex: position (3 × f32), uv (2 × f32), lightmap uv (2 × f32) (as the park packs).
pub const STRIDE: usize = crate::map::STRIDE;
/// A draw without albedo or lightmap texture.
pub const NO_TEXTURE: u32 = u32::MAX;
/// A lightmap slot that means "a mid grey": the pieces merged without their lightmap.
pub const GREY_LIGHTMAP: u32 = u32::MAX - 1;

/// Triangles of one material in a tile mesh: a range of its indices, which refer to the
/// mesh's own vertices from 0.
#[derive(Clone, Debug, PartialEq)]
pub struct TileDraw {
    pub first_index: u32,
    pub index_count: u32,
    /// Indices into the district's texture table (`NO_TEXTURE`: none).
    pub albedo: u32,
    pub lightmap: u32,
    pub color: [f32; 4],
    pub alpha_cutoff: Option<f32>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

/// Up to 65,536 vertices with their u16 indices and the draws over them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TileMesh {
    pub vertex_data: Vec<u8>,
    pub indices: Vec<u16>,
    pub draws: Vec<TileDraw>,
}

/// One level of detail of a tile: its meshes in a blob.
pub fn encode_lod(meshes: &[TileMesh]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u32(meshes.len() as u32);
    for m in meshes {
        w.u32(m.draws.len() as u32);
        for d in &m.draws {
            w.u32(d.first_index);
            w.u32(d.index_count);
            w.u32(d.albedo);
            w.u32(d.lightmap);
            for v in d.color.into_iter().chain(d.bounds_min).chain(d.bounds_max) {
                w.f32(v);
            }
            w.u8(u8::from(d.alpha_cutoff.is_some()));
            w.f32(d.alpha_cutoff.unwrap_or(0.0));
        }
        w.block(&m.vertex_data);
        w.u32(m.indices.len() as u32);
        for i in &m.indices {
            w.u16(*i);
        }
    }
    w.bytes
}

pub fn decode_lod(bytes: &[u8]) -> Result<Vec<TileMesh>, String> {
    let mut r = Reader::new(bytes);
    let mut meshes = Vec::new();
    for _ in 0..r.u32()? {
        let mut draws = Vec::new();
        for _ in 0..r.u32()? {
            let (first_index, index_count, albedo, lightmap) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let mut f = [0f32; 10];
            for v in &mut f {
                *v = r.f32()?;
            }
            let masked = r.u8()? != 0;
            let cutoff = r.f32()?;
            draws.push(TileDraw {
                first_index,
                index_count,
                albedo,
                lightmap,
                color: [f[0], f[1], f[2], f[3]],
                alpha_cutoff: masked.then_some(cutoff),
                bounds_min: [f[4], f[5], f[6]],
                bounds_max: [f[7], f[8], f[9]],
            });
        }
        let vertex_data = r.block()?.to_vec();
        let count = r.u32()? as usize;
        let raw = r.take(count * 2)?;
        let indices: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let vertices = (vertex_data.len() / STRIDE) as u32;
        if vertex_data.len() % STRIDE != 0 || indices.iter().any(|&i| u32::from(i) >= vertices) || draws.iter().any(|d| (d.first_index + d.index_count) as usize > indices.len()) {
            return Err("a tile mesh refers outside itself".into());
        }
        meshes.push(TileMesh { vertex_data, indices, draws });
    }
    r.done()?;
    Ok(meshes)
}

/// The index of a district pack, resident.
pub struct District {
    pub sim: SkateMap,
    pub authored_spawn: [f32; 3],
    pub authored_heading: f32,
    /// The whole world's broadphase paddings (`skate_sim::collision_window::broadphase_limits`).
    pub limits: (f32, f32),
    pub clusters: Vec<ClusterInfo>,
    /// Side of a render tile in metres.
    pub tile_size: f32,
    pub tiles: Vec<Tile>,
    /// (offset, size) of each texture in the table (`DistrictFile::read_texture`).
    pub textures: Vec<(u64, u32)>,
}

/// What the baker has: the index plus each blob's bytes.
pub struct DistrictBuilder {
    pub sim: SkateMap,
    pub authored_spawn: [f32; 3],
    pub authored_heading: f32,
    pub limits: (f32, f32),
    pub clusters: Vec<(ClusterInfo, Vec<u8>)>,
    pub tile_size: f32,
    pub tiles: Vec<(i32, i32, [Vec<u8>; 3])>,
    pub textures: Vec<crate::texture::Texture>,
}

impl DistrictBuilder {
    pub fn to_pack(&self) -> Vec<u8> {
        let mut blobs: Vec<u8> = Vec::new();
        let mut place = |bytes: &[u8]| {
            let at = blobs.len() as u64;
            blobs.extend_from_slice(bytes);
            (at, bytes.len() as u32)
        };
        let cluster_at: Vec<(u64, u32)> = self.clusters.iter().map(|(_, b)| place(b)).collect();
        let tile_at: Vec<[(u64, u32); 3]> = self.tiles.iter().map(|(_, _, l)| [place(&l[0]), place(&l[1]), place(&l[2])]).collect();
        let texture_at: Vec<(u64, u32)> = self
            .textures
            .iter()
            .map(|t| {
                let mut w = Writer::default();
                t.write(&mut w);
                place(&w.bytes)
            })
            .collect();
        let mut w = Writer::default();
        w.block(&crate::map::encode_sim(&self.sim));
        for v in self.authored_spawn {
            w.f32(v);
        }
        w.f32(self.authored_heading);
        w.f32(self.limits.0);
        w.f32(self.limits.1);
        w.u32(self.clusters.len() as u32);
        for ((info, _), (offset, size)) in self.clusters.iter().zip(&cluster_at) {
            for v in info.min.into_iter().chain(info.max) {
                w.f32(v);
            }
            w.u32(info.triangles);
            w.u64(*offset);
            w.u32(*size);
            w.f32(info.params.granularity);
            w.u8(info.params.group_width);
            w.u8(info.params.surface_width);
            w.u8(u8::from(info.params.one_sided));
        }
        w.f32(self.tile_size);
        w.u32(self.tiles.len() as u32);
        for ((cx, cz, _), lods) in self.tiles.iter().zip(&tile_at) {
            w.i32(*cx);
            w.i32(*cz);
            for (offset, size) in lods {
                w.u64(*offset);
                w.u32(*size);
            }
        }
        w.u32(self.textures.len() as u32);
        for (offset, size) in &texture_at {
            w.u64(*offset);
            w.u32(*size);
        }
        // payload = u32 index length, the index, the blobs.
        let mut payload = Vec::with_capacity(4 + w.bytes.len() + blobs.len());
        payload.extend_from_slice(&(w.bytes.len() as u32).to_le_bytes());
        payload.extend_from_slice(&w.bytes);
        payload.extend_from_slice(&blobs);
        crate::seal(Kind::District, VERSION, &payload)
    }
}

/// An open district pack: the file, for ranged reads.
pub struct DistrictFile {
    file: Mutex<std::fs::File>,
    /// Where it was opened ([`DistrictFile::reopen`]).
    path: std::path::PathBuf,
    /// File offset of the first blob.
    data_start: u64,
    file_len: u64,
    /// (offset, size) of each cluster's bytes.
    clusters: Vec<(u64, u32)>,
}

/// Opens a district pack reading only its header and index. The payload hash is not checked
/// (that would read the whole file): see [`verify_file`].
pub fn open(path: &Path) -> Result<(District, DistrictFile), String> {
    let name = path.display().to_string();
    let mut file = std::fs::File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let file_len = file.metadata().map_err(|e| format!("{name}: {e}"))?.len();
    let mut header = [0u8; HEADER_LEN + 4];
    file.read_exact(&mut header).map_err(|e| format!("{name}: {e}"))?;
    if header[..8] != MAGIC {
        return Err(format!("{name}: not a skate vita pack (bad magic)"));
    }
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    if word(8) != 4 || word(12) != VERSION {
        return Err(format!("{name}: expected a district pack v{VERSION}, found kind {} v{}", word(8), word(12)));
    }
    let length = u64::from_le_bytes(header[16..24].try_into().unwrap());
    if file_len != HEADER_LEN as u64 + length {
        return Err(format!("{name}: pack length does not match its header (truncated?)"));
    }
    let index_len = word(HEADER_LEN) as u64;
    if index_len + 4 > length {
        return Err(format!("{name}: bad index length"));
    }
    let mut index = vec![0u8; index_len as usize];
    file.read_exact(&mut index).map_err(|e| format!("{name}: {e}"))?;
    let data_start = HEADER_LEN as u64 + 4 + index_len;
    let blob_len = file_len - data_start;

    let mut r = Reader::new(&index);
    let sim = crate::map::decode_sim(r.block()?)?;
    let authored_spawn = [r.f32()?, r.f32()?, r.f32()?];
    let authored_heading = r.f32()?;
    let limits = (r.f32()?, r.f32()?);
    let count = r.u32()? as usize;
    let mut clusters = Vec::with_capacity(count);
    let mut located = Vec::with_capacity(count);
    for _ in 0..count {
        let min = [r.f32()?, r.f32()?, r.f32()?];
        let max = [r.f32()?, r.f32()?, r.f32()?];
        let triangles = r.u32()?;
        let (offset, size) = (r.u64()?, r.u32()?);
        let params = ClusterParams { granularity: r.f32()?, group_width: r.u8()?, surface_width: r.u8()?, one_sided: r.u8()? != 0 };
        if offset + u64::from(size) > blob_len {
            return Err(format!("{name}: a cluster lies outside the file"));
        }
        clusters.push(ClusterInfo { min, max, triangles, params });
        located.push((offset, size));
    }
    let tile_size = r.f32()?;
    let tile_count = r.u32()? as usize;
    let mut tiles = Vec::with_capacity(tile_count);
    for _ in 0..tile_count {
        let (cx, cz) = (r.i32()?, r.i32()?);
        let mut lods = [(0u64, 0u32); 3];
        for lod in &mut lods {
            *lod = (r.u64()?, r.u32()?);
            if lod.0 + u64::from(lod.1) > blob_len {
                return Err(format!("{name}: a tile lies outside the file"));
            }
        }
        tiles.push(Tile { cx, cz, lods });
    }
    let texture_count = r.u32()? as usize;
    let mut textures = Vec::with_capacity(texture_count);
    for _ in 0..texture_count {
        let (offset, size) = (r.u64()?, r.u32()?);
        if offset + u64::from(size) > blob_len {
            return Err(format!("{name}: a texture lies outside the file"));
        }
        textures.push((offset, size));
    }
    r.done()?;
    Ok((
        District { sim, authored_spawn, authored_heading, limits, clusters, tile_size, tiles, textures },
        DistrictFile { file: Mutex::new(file), path: path.to_path_buf(), data_start, file_len, clusters: located },
    ))
}

impl DistrictFile {
    /// The same pack on a handle of its own (its index shared, not read again):
    /// the collision windows read clusters on one while the render streams tiles and textures
    /// on the other, instead of queueing behind them on one lock.
    pub fn reopen(&self) -> Result<DistrictFile, String> {
        let file = std::fs::File::open(&self.path).map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(DistrictFile { file: Mutex::new(file), path: self.path.clone(), data_start: self.data_start, file_len: self.file_len, clusters: self.clusters.clone() })
    }

    /// `len` bytes at `offset` in the blob area (one seek, one read).
    /// A foreground read (`skate_data::io_gate`): the background pack checks,
    /// which read whole packs, wait while the game reads collision or render data.
    pub fn read_range(&self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        self.read_range_for("district", offset, len)
    }

    /// [`Self::read_range`] counted for `source` (`skate_data::io_gate::account`).
    pub fn read_range_for(&self, source: &str, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        let _foreground = skate_data::io_gate::foreground();
        let started = std::time::Instant::now();
        let mut out = vec![0u8; len as usize];
        let mut file = self.file.lock().map_err(|_| "district file lock poisoned".to_string())?;
        file.seek(SeekFrom::Start(self.data_start + offset)).map_err(|e| e.to_string())?;
        file.read_exact(&mut out).map_err(|e| e.to_string())?;
        skate_data::io_gate::account(source, len as usize, started.elapsed().as_micros() as u64);
        Ok(out)
    }

    /// One level of detail of a tile (empty when the tile has none at that level).
    pub fn read_tile(&self, tile: &Tile, lod: usize) -> Result<Vec<u8>, String> {
        let (offset, size) = *tile.lods.get(lod).ok_or("no such level of detail")?;
        if size == 0 {
            return Ok(Vec::new());
        }
        self.read_range_for("district tiles", offset, size)
    }

    /// Textures of the table in file order, neighbours within `TEXTURE_GAP` merged into one read each (a tile's
    /// textures mostly lie together; the card's small reads are slow when the streaming, the collision and the audio
    /// share it). Returned in the order asked.
    pub fn read_textures(&self, entries: &[(u64, u32)]) -> Result<Vec<crate::texture::Texture>, String> {
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&k| entries[k].0);
        let mut out: Vec<Option<crate::texture::Texture>> = (0..entries.len()).map(|_| None).collect();
        let mut i = 0;
        while i < order.len() {
            let start = entries[order[i]].0;
            let mut end = start + u64::from(entries[order[i]].1);
            let mut j = i + 1;
            // At most `TEXTURE_RUN` a read: one buffer of a few MB would need that much in one piece of the heap.
            while j < order.len() && entries[order[j]].0 <= end + TEXTURE_GAP && entries[order[j]].0 + u64::from(entries[order[j]].1) - start <= TEXTURE_RUN {
                end = end.max(entries[order[j]].0 + u64::from(entries[order[j]].1));
                j += 1;
            }
            let run = self.read_range_for("district textures", start, (end - start) as u32)?;
            for &k in &order[i..j] {
                let (o, s) = entries[k];
                let mut r = Reader::new(&run[(o - start) as usize..(o - start) as usize + s as usize]);
                let t = crate::texture::Texture::read(&mut r)?;
                r.done()?;
                t.validate()?;
                out[k] = Some(t);
            }
            i = j;
        }
        Ok(out.into_iter().flatten().collect())
    }

    /// A texture of the table.
    pub fn read_texture(&self, entry: (u64, u32)) -> Result<crate::texture::Texture, String> {
        let bytes = self.read_range_for("district textures", entry.0, entry.1)?;
        let mut r = Reader::new(&bytes);
        let t = crate::texture::Texture::read(&mut r)?;
        r.done()?;
        t.validate()?;
        Ok(t)
    }

    pub fn len(&self) -> u64 {
        self.file_len
    }
}

impl ClusterSource for DistrictFile {
    /// Reads the clusters in file order, merging neighbours into one read each.
    fn read(&self, ids: &[usize]) -> Result<Vec<Vec<u8>>, String> {
        let mut order: Vec<usize> = (0..ids.len()).collect();
        let at = |k: usize| self.clusters.get(ids[k]).copied().ok_or_else(|| format!("no cluster {}", ids[k]));
        for &k in &order {
            at(k)?;
        }
        order.sort_by_key(|&k| self.clusters[ids[k]].0);
        let mut out: Vec<Vec<u8>> = vec![Vec::new(); ids.len()];
        let mut i = 0;
        while i < order.len() {
            let (start, _) = at(order[i])?;
            let mut end = start + u64::from(at(order[i])?.1);
            let mut j = i + 1;
            while j < order.len() {
                let (o, s) = at(order[j])?;
                if o > end + MERGE_GAP {
                    break;
                }
                end = end.max(o + u64::from(s));
                j += 1;
            }
            let run = self.read_range_for("district collision", start, (end - start) as u32)?;
            for &k in &order[i..j] {
                let (o, s) = at(k)?;
                out[k] = run[(o - start) as usize..(o - start) as usize + s as usize].to_vec();
            }
            i = j;
        }
        Ok(out)
    }
}

/// Checks the payload hash by reading the file in pieces (a background check: 0.8 s per 25 MB of
/// blake3 on the Vita's CPU).
pub fn verify_file(path: &Path) -> Result<(), String> {
    let name = path.display().to_string();
    let mut file = std::fs::File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header).map_err(|e| format!("{name}: {e}"))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        // Out of the way of a loader reading from the card (skate_data::io_gate).
        skate_data::io_gate::wait_background(std::time::Duration::from_secs(1));
        let n = file.read(&mut buffer).map_err(|e| format!("{name}: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    if hasher.finalize().as_bytes() != &header[24..56] {
        return Err(format!("{name}: pack payload hash mismatch (damaged or truncated file)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_data::skate_map::{Geometry, SkateMap};

    fn sim() -> SkateMap {
        SkateMap {
            version: 14,
            name: "T".into(),
            spawn: [1.0, 2.0, 3.0],
            heading: 0.5,
            environment: vec![],
            materials: vec![],
            textures: vec![],
            geometry: Geometry { vertices: vec![], indices: vec![], collision: vec![] },
            rails: vec![],
            doors: vec![],
            lights: vec![],
            routes: vec![],
            extensions: vec![],
        }
    }

    fn info(i: u32) -> ClusterInfo {
        ClusterInfo {
            min: [i as f32, 0.0, 0.0],
            max: [i as f32 + 1.0, 1.0, 1.0],
            triangles: 10 + i,
            params: ClusterParams { granularity: 0.25, group_width: 1, surface_width: 2, one_sided: i % 2 == 1 },
        }
    }

    #[test]
    fn a_district_opens_by_its_index_and_reads_clusters_and_tiles_by_offset() {
        let clusters: Vec<(ClusterInfo, Vec<u8>)> = (0..40u32).map(|i| (info(i), vec![i as u8; 100 + i as usize])).collect();
        let builder = DistrictBuilder {
            sim: sim(),
            authored_spawn: [4.0, 5.0, 6.0],
            authored_heading: 1.5,
            limits: (0.0, 0.125),
            clusters: clusters.clone(),
            tile_size: 64.0,
            tiles: vec![(-1, 2, [vec![7; 50], vec![8; 20], Vec::new()]), (0, 0, [vec![9; 30], Vec::new(), Vec::new()])],
            textures: vec![crate::texture::Texture { width: 4, height: 4, format: crate::texture::Format::Dxt1, mips: vec![vec![1; 8], vec![2; 8], vec![3; 8]] }],
        };
        let path = std::env::temp_dir().join(format!("district-test-{}.pak", std::process::id()));
        std::fs::write(&path, builder.to_pack()).unwrap();
        let (district, file) = open(&path).unwrap();
        assert_eq!(district.sim.name, "T");
        assert_eq!((district.authored_spawn, district.authored_heading, district.limits), ([4.0, 5.0, 6.0], 1.5, (0.0, 0.125)));
        assert_eq!(district.clusters.len(), 40);
        assert_eq!(district.clusters[7], info(7));
        // Any order, with duplicates, far apart and neighbouring: each comes back as written.
        let ids = [39, 0, 7, 8, 7, 20, 1];
        let got = file.read(&ids).unwrap();
        for (id, bytes) in ids.iter().zip(&got) {
            assert_eq!(bytes, &clusters[*id].1);
        }
        assert!(file.read(&[40]).is_err());
        assert_eq!(district.tiles.len(), 2);
        assert_eq!((district.tiles[0].cx, district.tiles[0].cz), (-1, 2));
        assert_eq!(file.read_tile(&district.tiles[0], 0).unwrap(), vec![7; 50]);
        assert_eq!(file.read_tile(&district.tiles[0], 1).unwrap(), vec![8; 20]);
        assert!(file.read_tile(&district.tiles[0], 2).unwrap().is_empty());
        assert_eq!(file.read_texture(district.textures[0]).unwrap().mips[1], vec![2; 8]);
        // A tile mesh round-trips and refuses indices outside its vertices.
        let mesh = TileMesh {
            vertex_data: vec![0; STRIDE * 3],
            indices: vec![0, 1, 2],
            draws: vec![TileDraw { first_index: 0, index_count: 3, albedo: 0, lightmap: NO_TEXTURE, color: [1.0, 0.5, 0.25, 1.0], alpha_cutoff: Some(0.5), bounds_min: [0.0; 3], bounds_max: [1.0; 3] }],
        };
        assert_eq!(decode_lod(&encode_lod(&[mesh.clone()])).unwrap(), vec![mesh.clone()]);
        let mut bad = mesh;
        bad.indices[2] = 3;
        assert!(decode_lod(&encode_lod(&[bad])).is_err());
        verify_file(&path).unwrap();
        // A damaged byte is caught by the hash check, a truncated file by the length check.
        let mut bytes = std::fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        assert!(verify_file(&path).unwrap_err().contains("hash"));
        bytes.pop();
        std::fs::write(&path, &bytes).unwrap();
        assert!(open(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
