//! Little-endian SKATE01..15 reader. Layout follows the supplied
//! SK8R15/Source/tools/blender_owned_map/SKATE_FORMAT.md and
//! owned/world/src/owned_map_package.cpp. No geometry or physics is inferred.
use std::{io::Read, path::Path, time::Instant};
mod texture_decode;
mod storage_v15;

#[derive(Debug, PartialEq)]
pub struct SkateMap {
    pub version: u8,
    pub name: String,
    pub spawn: [f32; 3],
    pub heading: f32,
    /// Authored environment fields in wire order (12, 14, or 45 floats).
    pub environment: Vec<f32>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub geometry: Geometry,
    pub rails: Vec<Rail>,
    pub doors: Vec<Door>,
    pub lights: Vec<Light>,
    pub routes: Vec<Route>,
    pub extensions: Vec<Extension>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub flags: u32,
    pub friction: f32,
    pub restitution: f32,
    pub color: [f32; 3],
    pub roughness: f32,
    pub emissive: f32,
    /// Albedo, indirect, normal, ORM, emissive. One-based, zero absent.
    pub textures: [u32; 5],
    pub indirect_strength: f32,
    pub alpha_mode: u32,
    pub alpha_cutoff: f32,
    pub audio: u32,
    pub physics: u32,
    pub pattern: u32,
    pub depth_layer: Option<u32>,
    /// Complete v12+ retail definition bytes; distinct from portable PBR fields.
    pub retail_definition: Option<Vec<u8>>,
}
#[derive(Debug, PartialEq)]
pub struct Texture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub color_space: u32,
    pub rgba: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
    pub material: u32,
    pub decal_uv: Option<[f32; 2]>,
    pub tangent_frame: Option<[u8; 4]>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Collision {
    pub points: [[f32; 3]; 3],
    pub surface: u32,
    pub material: u32,
    pub native_edges: Option<[u8; 3]>,
}
#[derive(Debug, PartialEq)]
pub struct Geometry {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub collision: Vec<Collision>,
}
#[derive(Debug, PartialEq)]
pub struct Rail {
    pub name: String,
    pub closed: bool,
    pub points: Vec<[f32; 3]>,
    pub native: Option<Vec<u8>>,
}
#[derive(Debug, PartialEq)]
pub struct Door {
    pub name: String,
    /// Hinge position/axis, closed width/depth axes, local min/max.
    pub frame: [[f32; 3]; 6],
    /// Minimum/maximum/initial angle, mass, damping.
    pub motion: [f32; 5],
    /// Spring, max angular speed, contact impulse scale (absent in v4).
    pub response: Option<[f32; 3]>,
    pub friction: f32,
    pub restitution: f32,
    pub surface: u32,
    pub geometry: Geometry,
}
#[derive(Debug, PartialEq)]
pub struct Light {
    pub name: String,
    pub kind: u32,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
    pub radius: f32,
    pub inner_cos: f32,
    pub outer_cos: f32,
}
#[derive(Debug, PartialEq)]
pub struct Route {
    pub rail: Rail,
    pub skaters: u32,
    pub speed: f32,
    pub spacing: f32,
}
#[derive(Debug, PartialEq)]
pub struct Extension {
    pub tag: [u8; 4],
    pub schema: u32,
    pub payload: Vec<u8>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// A decoded MOBJ static-object record: identity plus a geometry range shared
/// with other instances of the same template.
/// MOBJ authored rigid-body parameters (schema 3+): the six-float physics
/// block plus the two sleep flags. `physics_type` and `collision_shape` stay
/// validated-only; dynamic bodies are built by the runtime, not authored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectPhysics {
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_scale: f32,
    pub enable_sleep: bool,
    pub initially_awake: bool,
}

impl Default for ObjectPhysics {
    fn default() -> Self {
        Self {
            density: 100.0,
            friction: 0.55,
            restitution: 0.05,
            linear_damping: 0.05,
            angular_damping: 0.15,
            gravity_scale: 1.0,
            enable_sleep: true,
            initially_awake: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaticObject {
    pub id: u32,
    pub name: String,
    /// Row-vector affine placement (v @ basis + translation): the three basis
    /// rows, then the translation. Schema 3 records carry an origin only, so
    /// their basis is the identity.
    pub transform: [f32; 12],
    pub first_index: u32,
    pub index_count: u32,
    pub first_collision: u32,
    pub collision_count: u32,
    pub rails: Vec<u32>,
    pub physics: ObjectPhysics,
}

/// MOBJ schema 3 stores editor ownership of ranges in the base geometry.
/// Physics-disabled objects can use those unchanged static arrays. Explicit
/// body types still require a runtime adapter and must not be flattened.
/// Schema 4 appends a 12-float row-vector affine transform per record so
/// instances can share one template geometry range with distinct placements.
pub fn parse_static_objects(map: &SkateMap, extension: &Extension) -> Result<Vec<StaticObject>, String> {
    if extension.schema != 3 && extension.schema != 4 { return Err("Unsupported MOBJ schema".into()); }
    let mut r = Reader { bytes: &extension.payload, at: 0 };
    let count = r.u()?;
    r.check_count(count, 80)?;
    let mut ids = std::collections::HashSet::new();
    let mut objects = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let id = r.u()?;
        if !ids.insert(id) { return Err("Duplicate MOBJ identity".into()); }
        let name = r.string()?;
        let origin = r.floats::<3>()?;
        let mut ranges = [0; 4];
        for (limit, slot) in [(map.geometry.indices.len(), 0), (map.geometry.collision.len(), 2)] {
            let first = r.u()? as usize;
            let length = r.u()? as usize;
            if first.checked_add(length).is_none_or(|end| end > limit) {
                return Err(format!("MOBJ {name} geometry range is invalid"));
            }
            ranges[slot] = first as u32;
            ranges[slot + 1] = length as u32;
        }
        let rails = r.u()?;
        r.check_count(rails, 4)?;
        let mut rail_ids = Vec::with_capacity(rails as usize);
        for _ in 0..rails {
            let rail = r.u()?;
            if rail as usize >= map.rails.len() { return Err(format!("MOBJ {name} rail index is invalid")); }
            rail_ids.push(rail);
        }
        if r.u()? != 0 {
            return Err(format!("MOBJ {name} requests object physics, which requires a body adapter"));
        }
        if r.u()? > 2 { return Err(format!("MOBJ {name} collision shape is invalid")); }
        let values = r.floats::<6>()?;
        let mut flags = [false; 2];
        for slot in &mut flags {
            let value = r.u()?;
            if value > 1 { return Err(format!("MOBJ {name} has an invalid boolean")); }
            *slot = value != 0;
        }
        let physics = ObjectPhysics {
            density: values[0],
            friction: values[1],
            restitution: values[2],
            linear_damping: values[3],
            angular_damping: values[4],
            gravity_scale: values[5],
            enable_sleep: flags[0],
            initially_awake: flags[1],
        };
        let transform = if extension.schema >= 4 {
            r.floats::<12>()?
        } else {
            [1., 0., 0., 0., 1., 0., 0., 0., 1., origin[0], origin[1], origin[2]]
        };
        objects.push(StaticObject {
            id,
            name,
            transform,
            first_index: ranges[0],
            index_count: ranges[1],
            first_collision: ranges[2],
            collision_count: ranges[3],
            rails: rail_ids,
            physics,
        });
    }
    if r.at != r.bytes.len() { return Err("MOBJ has trailing data".into()); }
    Ok(objects)
}

pub fn validate_static_objects(map: &SkateMap, extension: &Extension) -> Result<(), String> {
    parse_static_objects(map, extension).map(|_| ())
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("SKATE offset overflow")?;
        let result = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| format!("Truncated SKATE at byte {} (need {n})", self.at))?;
        self.at = end;
        Ok(result)
    }
    fn u(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f(&mut self) -> Result<f32, String> {
        let value = f32::from_bits(self.u()?);
        if !value.is_finite() {
            return Err(format!("Non-finite SKATE float at {}", self.at - 4));
        }
        Ok(value)
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let mut result = [0.; N];
        for v in &mut result {
            *v = self.f()?;
        }
        Ok(result)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "Invalid SKATE UTF-8".into())
    }
    fn check_count(&self, count: u32, minimum_bytes: usize) -> Result<(), String> {
        if count as usize > (self.bytes.len() - self.at) / minimum_bytes {
            return Err("SKATE count exceeds remaining data".into());
        }
        Ok(())
    }
    fn points(&mut self) -> Result<Vec<[f32; 3]>, String> {
        let n = self.u()?;
        self.check_count(n, 12)?;
        (0..n).map(|_| self.floats()).collect()
    }
    fn stored_block(&mut self, expected: usize) -> Result<StoredBlock<'a>, String> {
        if expected > 2_147_483_648 {
            return Err("SKATE decoded block exceeds 2 GiB reader limit".into());
        }
        let method = self.u()?;
        let n = self.u()? as usize;
        let bytes = self.take(n)?;
        Ok(StoredBlock { expected, method, bytes })
    }
    fn stored(&mut self, expected: usize) -> Result<Vec<u8>, String> {
        self.stored_block(expected)?.decode()
    }
    fn geometry(
        &mut self,
        counts: [u32; 3],
        materials: usize,
        version: u8,
    ) -> Result<Geometry, String> {
        let [nv, ni, nc] = counts;
        self.check_count(nv, if version >= 12 { 56 } else { 44 })?;
        let vertices: Vec<Vertex> = (0..nv)
            .map(|_| {
                Ok(Vertex {
                    position: self.floats()?,
                    normal: self.floats()?,
                    uv: self.floats()?,
                    lightmap_uv: self.floats()?,
                    material: self.u()?,
                    decal_uv: if version >= 12 {
                        Some(self.floats()?)
                    } else {
                        None
                    },
                    tangent_frame: if version >= 12 {
                        Some(self.take(4)?.try_into().unwrap())
                    } else {
                        None
                    },
                })
            })
            .collect::<Result<_, String>>()?;
        self.check_count(ni, 4)?;
        let indices: Vec<u32> = (0..ni).map(|_| self.u()).collect::<Result<_, _>>()?;
        self.check_count(nc, if version >= 11 { 48 } else { 44 })?;
        let collision: Vec<Collision> = (0..nc)
            .map(|_| {
                let points = [self.floats()?, self.floats()?, self.floats()?];
                let surface = self.u()?;
                let material = self.u()?;
                let native_edges = if version >= 11 {
                    let bytes = self.take(4)?;
                    if bytes[3] != 0 {
                        Some(bytes[..3].try_into().unwrap())
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok(Collision {
                    points,
                    surface,
                    material,
                    native_edges,
                })
            })
            .collect::<Result<_, String>>()?;
        let valid_material = |id: u32| id > 0 && id as usize <= materials;
        if vertices.iter().any(|v| !valid_material(v.material))
            || collision
                .iter()
                .any(|v| !valid_material(v.material) || v.surface == 0)
        {
            return Err("SKATE invalid material/surface reference".into());
        }
        if indices.len() % 3 != 0 || indices.iter().any(|&i| i as usize >= vertices.len()) {
            return Err("SKATE invalid triangle indices".into());
        }
        for tri in indices.chunks_exact(3) {
            if tri
                .iter()
                .any(|&i| vertices[i as usize].material != vertices[tri[0] as usize].material)
            {
                return Err("SKATE triangle mixes materials".into());
            }
        }
        for tri in &collision {
            let [a, b, c] = tri.points;
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let area = cross.iter().map(|x| x * x).sum::<f32>();
            if !area.is_finite() || area <= 0. {
                return Err("SKATE degenerate collision triangle".into());
            }
        }
        Ok(Geometry {
            vertices,
            indices,
            collision,
        })
    }
}
/// Borrow compressed payloads while scanning; decompression does not mutate
/// the package or depend on any other texture.
#[derive(Clone, Copy)]
struct StoredBlock<'a> { expected: usize, method: u32, bytes: &'a [u8] }
impl StoredBlock<'_> {
    /// Rejects a declared size the stored bytes could never decode to, before
    /// anything is reserved for it. Deflate writes at most 1032 bytes per stored
    /// byte (two bits per 258-byte match), Zstandard at most 32768 (four bytes
    /// per RLE block of up to 128 KiB). A SKATE15 transform writes at most four
    /// bytes per byte of its payload (an index varint is at least one byte), and
    /// the payload follows a 4-byte size as Zstandard (3..=6) or deflate (7..=10).
    fn check_size(&self) -> Result<(), String> {
        let most = |method: u32, stored: usize| match method {
            0 => stored,
            1 => stored.saturating_mul(1032),
            _ => stored.saturating_mul(32768),
        };
        let limit = match self.method {
            0..=2 => most(self.method, self.bytes.len()),
            3..=10 => most(if self.method <= 6 { 2 } else { 1 }, self.bytes.len().saturating_sub(4)).saturating_mul(4),
            _ => return Ok(()),
        };
        if self.expected > limit {
            return Err(format!(
                "SKATE block declares {} bytes, more than its {} stored bytes can decode to",
                self.expected,
                self.bytes.len()
            ));
        }
        Ok(())
    }
    fn decode(&self) -> Result<Vec<u8>, String> {
        self.check_size()?;
        if (3..=10).contains(&self.method) {
            return storage_v15::decode(self.method, self.bytes, self.expected);
        }
        let mut decoded = exact_buffer(self.expected)?;
        self.decode_into(&mut decoded)?;
        Ok(decoded)
    }
    /// Appends the block's `expected` decoded bytes to `out`, which should
    /// already have room for them.
    fn decode_into(&self, out: &mut Vec<u8>) -> Result<(), String> {
        self.check_size()?;
        let Self { expected, method, bytes } = *self;
        let start = out.len();
        let ended = match method {
            0 => {
                let ended = bytes.len() == expected;
                if ended {
                    out.extend_from_slice(bytes);
                }
                ended
            }
            1 => {
                let mut decoder = flate2::read::ZlibDecoder::new(bytes);
                let ended = read_declared(&mut decoder, expected, out)
                    .map_err(|e| format!("SKATE DEFLATE: {e}"))?;
                if ended && decoder.total_in() as usize != bytes.len() {
                    return Err("SKATE compressed block has unused input".into());
                }
                ended
            }
            2 => {
                let mut decoder = zstd::stream::read::Decoder::new(bytes).map_err(|e| e.to_string())?;
                read_declared(&mut decoder, expected, out).map_err(|e| format!("SKATE Zstandard: {e}"))?
            }
            3..=10 => {
                out.extend_from_slice(&storage_v15::decode(method, bytes, expected)?);
                true
            }
            _ => return Err(format!("Unsupported SKATE storage method {method}")),
        };
        if !ended || out.len() - start != expected {
            return Err("SKATE decoded block size mismatch".into());
        }
        Ok(())
    }
}
/// An empty buffer with room for exactly `len` bytes. Growing one from a
/// smaller start doubles past the final size while the old buffer is still
/// alive: a 140.3 MiB vertex block asked for 256 MiB and failed on a 3 GiB heap.
fn exact_buffer(len: usize) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(len)
        .map_err(|_| format!("SKATE decoded block of {len} bytes cannot be reserved"))?;
    Ok(buffer)
}
/// Reads at most `expected` bytes into `out`, then reports whether the stream
/// ends there, so a block that decodes longer never grows `out`.
fn read_declared(reader: &mut impl Read, expected: usize, out: &mut Vec<u8>) -> std::io::Result<bool> {
    reader.by_ref().take(expected as u64).read_to_end(out)?;
    Ok(reader.read(&mut [0; 1])? == 0)
}
/// Decodes `blocks` back to back into one buffer reserved at their joined
/// size. Extending one block's exact buffer with the next would regrow it to
/// about twice its size while the old one is still alive.
fn decode_joined(blocks: &[StoredBlock<'_>]) -> Result<Vec<u8>, String> {
    let mut total = 0usize;
    for block in blocks {
        block.check_size()?;
        total = total.checked_add(block.expected).ok_or("SKATE geometry blocks are too large")?;
    }
    let mut joined = exact_buffer(total)?;
    for block in blocks {
        block.decode_into(&mut joined)?;
    }
    Ok(joined)
}
impl SkateMap {
    pub fn load(path: &Path) -> Result<Self, String> {
        let started = Instant::now();
        let data = crate::vfs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let disk = started.elapsed();
        let parse_started = Instant::now();
        let map = Self::parse(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("MAP_READ_TIMING name={:?} disk_ms={} parse_ms={} file_bytes={}",
            map.name, disk.as_millis(), parse_started.elapsed().as_millis(), data.len());
        Ok(map)
    }
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
        Self::parse_with_decode_workers(data, workers)
    }
    /// Data tools can request one worker for reproducible serial comparisons.
    /// The decoder caps concurrency at eight and skips threading for small maps.
    pub fn parse_with_decode_workers(data: &[u8], workers: usize) -> Result<Self, String> {
        Self::parse_inner(data, workers, true)
    }

    /// Decode a supplemental presentation package without requiring collision.
    /// Playable map loading continues to use `parse`/`load`. Static MOBJ
    /// instance records are presentation data here: they carry no collision
    /// until a physics adapter exists.
    pub fn parse_render_only(data: &[u8]) -> Result<Self, String> {
        let map = Self::parse_inner(data, 4, false)?;
        if !map.geometry.collision.is_empty() || !map.rails.is_empty()
            || !map.doors.is_empty() || !map.lights.is_empty() || !map.routes.is_empty()
            || map.extensions.iter().any(|e| e.tag != *b"WMET" && e.tag != *b"MOBJ")
        {
            return Err("SKATE render-only package contains non-presentation data".into());
        }
        for extension in map.extensions.iter().filter(|e| e.tag == *b"MOBJ") {
            validate_static_objects(&map, extension)?;
        }
        Ok(map)
    }
    fn parse_inner(data: &[u8], workers: usize, require_collision: bool) -> Result<Self, String> {
        let parse_started = Instant::now();
        let mut r = Reader { bytes: data, at: 0 };
        let magic = r.take(8)?;
        if &magic[..5] != b"SKATE"
            || magic[7] != 0
            || !magic[5].is_ascii_digit()
            || !magic[6].is_ascii_digit()
        {
            return Err("Invalid SKATE magic/version".into());
        }
        let version = (magic[5] - b'0') * 10 + magic[6] - b'0';
        if !(1..=15).contains(&version) {
            return Err("Unsupported SKATE version: reader supports 01 through 15; newer packages require an update".into());
        }
        if r.u()? != 0x12345678 {
            return Err("Invalid SKATE endian marker".into());
        }
        let name = r.string()?;
        if name.is_empty() {
            return Err("Empty SKATE map name".into());
        }
        let spawn = r.floats()?;
        let heading = r.f()?;
        let ne = if version >= 6 {
            45
        } else if version >= 3 {
            14
        } else {
            12
        };
        let environment = (0..ne).map(|_| r.f()).collect::<Result<_, _>>()?;
        let mut counts = [0; 9];
        let n = if version >= 8 {
            9
        } else if version >= 7 {
            8
        } else if version >= 4 {
            7
        } else {
            6
        };
        for c in &mut counts[..n] {
            *c = r.u()?;
        }
        let material_bytes = if version >= 15 {
            let size = r.u()? as usize;
            Some(r.stored(size)?)
        } else { None };
        let mut package_reader = None;
        if let Some(bytes) = &material_bytes {
            package_reader = Some(r);
            r = Reader { bytes, at: 0 };
        }
        r.check_count(counts[0], if version >= 2 { 80 } else { 48 })?;
        let mut materials = Vec::new();
        for _ in 0..counts[0] {
            let mut m = Material {
                name: r.string()?,
                flags: r.u()?,
                friction: r.f()?,
                restitution: r.f()?,
                color: r.floats()?,
                roughness: r.f()?,
                emissive: r.f()?,
                textures: [r.u()?, r.u()?, 0, 0, 0],
                indirect_strength: r.f()?,
                alpha_mode: 0,
                alpha_cutoff: 0.5,
                audio: 3,
                physics: 1,
                pattern: 0,
                depth_layer: None,
                retail_definition: None,
            };
            if version >= 2 {
                for id in &mut m.textures[2..] {
                    *id = r.u()?;
                }
                m.alpha_mode = r.u()?;
                m.alpha_cutoff = r.f()?;
                m.audio = r.u()?;
                m.physics = r.u()?;
                m.pattern = r.u()?;
            }
            if version >= 13 {
                let layer = r.u()?;
                if layer > 3 {
                    return Err("Invalid SKATE depth layer".into());
                }
                m.depth_layer = Some(layer);
            }
            if version >= 12 && r.u()? != 0 {
                let start = r.at;
                r.take(16)?;
                r.string()?;
                r.take(8)?;
                let bindings = r.u()?;
                r.check_count(bindings, 20)?;
                for _ in 0..bindings {
                    r.string()?;
                    let id = r.u()?;
                    let uv = r.u()?;
                    let au = r.u()?;
                    let av = r.u()?;
                    if id > counts[1] || uv > 2 || au > 1 || av > 1 {
                        return Err("Invalid SKATE retail texture binding".into());
                    }
                }
                let parameters = r.u()?;
                r.check_count(parameters, 8)?;
                for _ in 0..parameters {
                    r.string()?;
                    let n = r.u()?;
                    r.check_count(n, 4)?;
                    for _ in 0..n {
                        r.string()?;
                    }
                }
                r.string()?;
                m.retail_definition = Some(r.bytes[start..r.at].to_vec());
            }
            if m.name.is_empty()
                || m.textures.iter().any(|&id| id > counts[1])
                || m.alpha_mode > 2
                || m.audio > 127
                || m.physics > 13
                || m.pattern > 15
                || m.friction < 0.
                || m.restitution < 0.
                || !(0.0..=1.0).contains(&m.roughness)
                || !(0.0..=1.0).contains(&m.alpha_cutoff)
                || m.indirect_strength < 0.
            {
                return Err("Invalid SKATE material".into());
            }
            materials.push(m);
        }
        if let Some(package) = package_reader {
            if r.at != r.bytes.len() { return Err("SKATE material block has trailing bytes".into()); }
            r = package;
        }
        let texture_started = Instant::now();
        r.check_count(counts[1], 20)?;
        let mut textures = Vec::new();
        let mut texture_blocks: Vec<StoredBlock<'_>> = Vec::new();
        for _ in 0..counts[1] {
            let name = r.string()?;
            let width = r.u()?;
            let height = r.u()?;
            let color_space = r.u()?;
            if (width == 0) != (height == 0) || width > 16384 || height > 16384 || color_space > 1 {
                return Err("Invalid SKATE embedded texture".into());
            }
            let expected = width as usize * height as usize * 4;
            let block = if version >= 9 {
                r.stored_block(expected)?
            } else {
                let bytes = r.u()? as usize;
                if bytes != expected {
                    return Err("Invalid SKATE embedded texture size".into());
                }
                StoredBlock { expected, method: 0, bytes: r.take(bytes)? }
            };
            let block = if version >= 15 && block.method == 11 {
                let index = u32::from_le_bytes(block.bytes.try_into()
                    .map_err(|_| "Invalid SKATE texture reference size")?) as usize;
                let source = *texture_blocks.get(index).ok_or("Invalid SKATE forward texture reference")?;
                if source.expected != expected { return Err("SKATE texture reference size mismatch".into()); }
                source
            } else { block };
            texture_blocks.push(block);
            textures.push(Texture {
                name,
                width,
                height,
                color_space,
                rgba: Vec::new(),
            });
        }
        let texture_workers = texture_decode::decode(&mut textures, &texture_blocks, workers)?;
        let texture_time = texture_started.elapsed();
        let geometry_started = Instant::now();
        let geometry_counts = [counts[2], counts[3], counts[4]];
        let geometry = if version >= 9 {
            let blocks = [
                r.stored_block(counts[2] as usize * if version >= 12 { 56 } else { 44 })?,
                r.stored_block(counts[3] as usize * 4)?,
                r.stored_block(counts[4] as usize * if version >= 11 { 48 } else { 44 })?,
            ];
            let bytes = decode_joined(&blocks)?;
            Reader {
                bytes: &bytes,
                at: 0,
            }
            .geometry(geometry_counts, materials.len(), version)?
        } else {
            r.geometry(geometry_counts, materials.len(), version)?
        };
        let geometry_time = geometry_started.elapsed();
        if materials.is_empty()
            || geometry.vertices.is_empty()
            || geometry.indices.is_empty()
        {
            return Err("SKATE requires materials and render geometry".into());
        }
        r.check_count(counts[5], 12)?;
        let mut rails = Vec::new();
        for _ in 0..counts[5] {
            let name = r.string()?;
            let closed = r.u()? != 0;
            let representation = if version >= 10 { r.u()? } else { 0 };
            let (points, native) = match representation {
                0 => (r.points()?, None),
                1 => {
                    let start = r.at;
                    r.take(24)?;
                    let n = r.u()?;
                    r.check_count(n, 120)?;
                    r.take(n as usize * 120)?;
                    (vec![], Some(r.bytes[start..r.at].to_vec()))
                }
                _ => return Err("Unsupported SKATE rail representation".into()),
            };
            rails.push(Rail {
                name,
                closed,
                points,
                native,
            });
        }
        r.check_count(counts[6], 120)?;
        let mut doors = Vec::new();
        for _ in 0..counts[6] {
            let name = r.string()?;
            let frame = [
                r.floats()?,
                r.floats()?,
                r.floats()?,
                r.floats()?,
                r.floats()?,
                r.floats()?,
            ];
            let motion = r.floats()?;
            let response = if version >= 5 {
                Some(r.floats()?)
            } else {
                None
            };
            let friction = r.f()?;
            let restitution = r.f()?;
            let surface = r.u()?;
            let counts = [r.u()?, r.u()?, r.u()?];
            doors.push(Door {
                name,
                frame,
                motion,
                response,
                friction,
                restitution,
                surface,
                geometry: r.geometry(counts, materials.len(), version)?,
            });
        }
        r.check_count(counts[7], 64)?;
        let mut lights = Vec::new();
        for _ in 0..counts[7] {
            let light = Light {
                name: r.string()?,
                kind: r.u()?,
                position: r.floats()?,
                direction: r.floats()?,
                color: r.floats()?,
                intensity: r.f()?,
                range: r.f()?,
                radius: r.f()?,
                inner_cos: r.f()?,
                outer_cos: r.f()?,
            };
            if light.kind > 2
                || light.range <= 0.
                || light.intensity < 0.
                || light.radius < 0.
                || !(-1.0..=1.0).contains(&light.inner_cos)
                || !(-1.0..=1.0).contains(&light.outer_cos)
            {
                return Err("Invalid SKATE light".into());
            }
            lights.push(light);
        }
        r.check_count(counts[8], 24)?;
        let mut routes = Vec::new();
        for _ in 0..counts[8] {
            let name = r.string()?;
            let closed = r.u()? != 0;
            let skaters = r.u()?;
            let speed = r.f()?;
            let spacing = r.f()?;
            let points = r.points()?;
            routes.push(Route {
                rail: Rail {
                    name,
                    closed,
                    points,
                    native: None,
                },
                skaters,
                speed,
                spacing,
            });
        }
        let extension_started = Instant::now();
        let mut extensions = Vec::new();
        if version >= 12 {
            let n = r.u()?;
            r.check_count(n, 20)?;
            for _ in 0..n {
                let tag = r.take(4)?.try_into().unwrap();
                let schema = r.u()?;
                let bytes = r.u()? as usize;
                extensions.push(Extension {
                    tag,
                    schema,
                    payload: r.stored(bytes)?,
                });
            }
        }
        if require_collision && geometry.collision.is_empty() && !extensions.iter().any(|e| e.tag == *b"RWCM" && e.schema == 1 && !e.payload.is_empty()) {
            return Err("SKATE requires triangle collision or an embedded RWCM archive".into());
        }
        if r.at != data.len() {
            return Err(format!(
                "SKATE contains {} unexplained trailing bytes",
                data.len() - r.at
            ));
        }
        eprintln!("MAP_DECODE_TIMING name={:?} textures_ms={} texture_workers={} geometry_ms={} extensions_ms={} total_ms={}",
            name, texture_time.as_millis(), texture_workers, geometry_time.as_millis(),
            extension_started.elapsed().as_millis(), parse_started.elapsed().as_millis());
        Ok(Self {
            version,
            name,
            spawn,
            heading,
            environment,
            materials,
            textures,
            geometry,
            rails,
            doors,
            lights,
            routes,
            extensions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn map(indices: u32) -> SkateMap {
        SkateMap {
            version: 14,
            name: "props".into(),
            spawn: [0.; 3],
            heading: 0.,
            environment: vec![0.; 45],
            materials: vec![],
            textures: vec![],
            geometry: Geometry {
                vertices: vec![],
                indices: vec![0; indices as usize],
                collision: vec![],
            },
            rails: vec![],
            doors: vec![],
            lights: vec![],
            routes: vec![],
            extensions: vec![],
        }
    }

    fn u32s(payload: &mut Vec<u8>, values: &[u32]) {
        for v in values {
            payload.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn floats(payload: &mut Vec<u8>, values: &[f32]) {
        for v in values {
            payload.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn record(payload: &mut Vec<u8>, id: u32, name: &str, range: [u32; 2], affine: Option<[f32; 12]>) {
        u32s(payload, &[id, name.len() as u32]);
        payload.extend_from_slice(name.as_bytes());
        floats(payload, &[10., 20., 30.]); // origin
        u32s(payload, &[range[0], range[1], 0, 0, 0]); // render range, collision range, no rails
        u32s(payload, &[0, 0]); // no physics, collision shape
        floats(payload, &[100., 0.55, 0.05, 0.05, 0.15, 1.]);
        u32s(payload, &[1, 0]);
        if let Some(affine) = affine {
            floats(payload, &affine);
        }
    }

    fn stored(method: u32, data: &[u8]) -> Vec<u8> {
        match method {
            0 => data.to_vec(),
            1 => {
                let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(data).unwrap();
                encoder.finish().unwrap()
            }
            _ => zstd::stream::encode_all(data, 1).unwrap(),
        }
    }

    #[test]
    fn mobj_schema4_instances_share_geometry_ranges() {
        let mut payload = Vec::new();
        u32s(&mut payload, &[2]);
        let affine = [0., 0., -1., 0., 1., 0., 1., 0., 0., 10., 20., 30.];
        record(&mut payload, 7, "0000000000000042/ramp_a", [12, 36], Some(affine));
        record(&mut payload, 8, "0000000000000042/ramp_b", [12, 36], Some(affine));
        let extension = Extension { tag: *b"MOBJ", schema: 4, payload };
        let objects = parse_static_objects(&map(48), &extension).unwrap();
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].id, 7);
        assert_eq!(objects[0].name, "0000000000000042/ramp_a");
        assert_eq!(objects[0].transform, affine);
        assert_eq!((objects[0].first_index, objects[0].index_count), (12, 36));
        assert_eq!(objects[1].first_index, objects[0].first_index);
    }

    #[test]
    fn mobj_schema3_transform_is_origin_only() {
        let mut payload = Vec::new();
        u32s(&mut payload, &[1]);
        record(&mut payload, 3, "bench", [0, 12], None);
        let extension = Extension { tag: *b"MOBJ", schema: 3, payload };
        let objects = parse_static_objects(&map(12), &extension).unwrap();
        assert_eq!(objects[0].transform, [1., 0., 0., 0., 1., 0., 0., 0., 1., 10., 20., 30.]);
    }

    #[test]
    fn mobj_rejects_out_of_range_and_physics() {
        let mut payload = Vec::new();
        u32s(&mut payload, &[1]);
        record(&mut payload, 1, "bad", [40, 12], None);
        let extension = Extension { tag: *b"MOBJ", schema: 3, payload };
        assert!(parse_static_objects(&map(48), &extension)
            .unwrap_err()
            .contains("geometry range"));

        let mut payload = Vec::new();
        u32s(&mut payload, &[1]);
        u32s(&mut payload, &[1, 4]);
        payload.extend_from_slice(b"door");
        floats(&mut payload, &[0.; 3]);
        u32s(&mut payload, &[0, 12, 0, 0, 0]);
        u32s(&mut payload, &[1, 0]); // physics body requested
        floats(&mut payload, &[100., 0.55, 0.05, 0.05, 0.15, 1.]);
        u32s(&mut payload, &[1, 0]);
        let extension = Extension { tag: *b"MOBJ", schema: 3, payload };
        assert!(parse_static_objects(&map(48), &extension)
            .unwrap_err()
            .contains("body adapter"));
    }

    #[test]
    fn blocks_decode_into_exactly_their_declared_size() {
        // Past the 8 MiB the decoder used to start from before doubling.
        let data: Vec<u8> = (0..9u32 << 20).map(|i| (i % 251) as u8).collect();
        for method in 0..=2 {
            let bytes = stored(method, &data);
            let decoded = StoredBlock { expected: data.len(), method, bytes: &bytes }.decode().unwrap();
            assert_eq!(decoded, data);
            assert_eq!(decoded.capacity(), data.len(), "method {method} regrew its buffer");
        }
    }

    #[test]
    fn a_block_that_decodes_longer_fails_without_growing_its_buffer() {
        for method in 0..=2 {
            let bytes = stored(method, &[7; 4096]);
            let mut out = exact_buffer(1024).unwrap();
            assert!(StoredBlock { expected: 1024, method, bytes: &bytes }.decode_into(&mut out).is_err());
            assert_eq!(out.capacity(), 1024, "method {method}");
        }
    }

    #[test]
    fn geometry_blocks_decode_into_one_buffer_reserved_once() {
        let parts: [Vec<u8>; 3] = [(0..60_000u32).map(|i| (i % 253) as u8).collect(), vec![3; 6_000], vec![9; 4_800]];
        let methods = [1, 2, 0];
        let stored: Vec<Vec<u8>> = parts.iter().zip(methods).map(|(part, method)| stored(method, part)).collect();
        let blocks: Vec<StoredBlock<'_>> = parts.iter().zip(&stored).zip(methods)
            .map(|((part, bytes), method)| StoredBlock { expected: part.len(), method, bytes })
            .collect();
        let joined = decode_joined(&blocks).unwrap();
        assert_eq!(joined, parts.concat());
        // Extending the first block's buffer would have left it about twice its size.
        assert_eq!(joined.capacity(), joined.len());
    }

    #[test]
    fn a_tiny_block_declaring_1_gib_fails_without_reserving_it() {
        for method in 0..=10 {
            let bytes = stored(method.min(2), &[0; 16]);
            let block = StoredBlock { expected: 1 << 30, method, bytes: &bytes };
            let mut out = Vec::new();
            let error = block.decode_into(&mut out).unwrap_err();
            assert!(error.contains("declares"), "method {method}: {error}");
            assert_eq!(out.capacity(), 0, "method {method}");
            assert!(block.decode().unwrap_err().contains("declares"), "method {method}");
            assert!(decode_joined(&[block]).unwrap_err().contains("declares"), "method {method}");
        }
    }

    /// Regression check for the decode changes (regression-check 3b): every installed map's geometry fingerprint.
    /// `SKATE3_MAPS_DIR=<install>/maps cargo test -p skate-data --lib installed_map_geometry_fingerprints -- --ignored
    /// --nocapture`, once per decoder version; the printed lines must be identical.
    #[test]
    #[ignore = "Requires installed maps"]
    fn installed_map_geometry_fingerprints() {
        use std::hash::{Hash, Hasher};
        let dir = std::path::PathBuf::from(std::env::var("SKATE3_MAPS_DIR").expect("SKATE3_MAPS_DIR"));
        let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "skate")).collect();
        paths.sort();
        for path in paths {
            let map = SkateMap::load(&path).unwrap();
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for v in &map.geometry.vertices {
                v.position.map(f32::to_bits).hash(&mut h);
                v.normal.map(f32::to_bits).hash(&mut h);
                v.uv.map(f32::to_bits).hash(&mut h);
                v.lightmap_uv.map(f32::to_bits).hash(&mut h);
                (v.material, v.decal_uv.map(|d| d.map(f32::to_bits)), v.tangent_frame).hash(&mut h);
            }
            map.geometry.indices.hash(&mut h);
            for c in &map.geometry.collision {
                (c.points.map(|p| p.map(f32::to_bits)), c.surface, c.material, c.native_edges).hash(&mut h);
            }
            println!("GEOMETRY {} v={} i={} c={} {:016x}", path.file_name().unwrap().to_string_lossy(),
                map.geometry.vertices.len(), map.geometry.indices.len(), map.geometry.collision.len(), h.finish());
        }
    }
}
