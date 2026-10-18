//! skate_vita: the movable props' package for the simulation on the Vita. The PC game
//! reads `private/native-props/<map>.skate` whole (render-only `.skate`: textures, geometry, the
//! MOBJ placements); the props' collision is built from the template geometry, so the Vita keeps
//! that and the materials and leaves the texture pixels out (DownTown's are 26 MB as RGBA).
//! `vita-bake sim` writes `<map>.props` next to where the `.skate` would be; `decode` gives the
//! `SkateMap` the loader expects, with empty textures and only positions and materials in the
//! vertices.
use crate::skate_map::{Extension, Geometry, Material, SkateMap, Texture, Vertex};

const MAGIC: &[u8; 4] = b"SKPR";
const VERSION: u32 = 1;

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len()).ok_or("prop package cut short")?;
        let b = &self.data[self.at..end];
        self.at = end;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?).map_err(|_| "prop package: bad text".to_owned())
    }
    fn count(&mut self, item: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n.saturating_mul(item) > self.data.len() - self.at {
            return Err("prop package: bad count".into());
        }
        Ok(n)
    }
}

/// The package of a render-only `.skate` of props (materials, positions, materials of the
/// vertices, indices, extensions; texture names and sizes without pixels).
pub fn encode(map: &SkateMap) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.0.extend_from_slice(MAGIC);
    w.u32(VERSION);
    w.u8(map.version);
    w.str(&map.name);
    w.u32(map.materials.len() as u32);
    for m in &map.materials {
        w.str(&m.name);
        w.u32(m.flags);
        w.f32(m.friction);
        w.f32(m.restitution);
        m.color.iter().for_each(|&v| w.f32(v));
        w.f32(m.roughness);
        w.f32(m.emissive);
        m.textures.iter().for_each(|&v| w.u32(v));
        w.f32(m.indirect_strength);
        w.u32(m.alpha_mode);
        w.f32(m.alpha_cutoff);
        w.u32(m.audio);
        w.u32(m.physics);
        w.u32(m.pattern);
        match m.depth_layer {
            Some(v) => {
                w.u8(1);
                w.u32(v);
            }
            None => w.u8(0),
        }
        match &m.retail_definition {
            Some(b) => {
                w.u8(1);
                w.bytes(b);
            }
            None => w.u8(0),
        }
    }
    w.u32(map.textures.len() as u32);
    for t in &map.textures {
        w.str(&t.name);
        w.u32(t.width);
        w.u32(t.height);
        w.u32(t.color_space);
    }
    w.u32(map.geometry.vertices.len() as u32);
    for v in &map.geometry.vertices {
        v.position.iter().for_each(|&x| w.f32(x));
        w.u32(v.material);
    }
    w.u32(map.geometry.indices.len() as u32);
    map.geometry.indices.iter().for_each(|&i| w.u32(i));
    w.u32(map.extensions.len() as u32);
    for e in &map.extensions {
        w.0.extend_from_slice(&e.tag);
        w.u32(e.schema);
        w.bytes(&e.payload);
    }
    w.0
}

/// Whether `bytes` are a prop package (rather than a `.skate`).
pub fn is_package(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

pub fn decode(bytes: &[u8]) -> Result<SkateMap, String> {
    let mut r = Reader { data: bytes, at: 0 };
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return Err("not a prop package of this version".into());
    }
    let version = r.u8()?;
    let name = r.str()?;
    let mut materials = Vec::new();
    for _ in 0..r.count(60)? {
        materials.push(Material {
            name: r.str()?,
            flags: r.u32()?,
            friction: r.f32()?,
            restitution: r.f32()?,
            color: [r.f32()?, r.f32()?, r.f32()?],
            roughness: r.f32()?,
            emissive: r.f32()?,
            textures: [r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?],
            indirect_strength: r.f32()?,
            alpha_mode: r.u32()?,
            alpha_cutoff: r.f32()?,
            audio: r.u32()?,
            physics: r.u32()?,
            pattern: r.u32()?,
            depth_layer: if r.u8()? == 1 { Some(r.u32()?) } else { None },
            retail_definition: if r.u8()? == 1 { Some(r.bytes()?) } else { None },
        });
    }
    let mut textures = Vec::new();
    for _ in 0..r.count(16)? {
        textures.push(Texture { name: r.str()?, width: r.u32()?, height: r.u32()?, color_space: r.u32()?, rgba: Vec::new() });
    }
    let mut vertices = Vec::new();
    for _ in 0..r.count(16)? {
        let position = [r.f32()?, r.f32()?, r.f32()?];
        let material = r.u32()?;
        vertices.push(Vertex { position, normal: [0.0; 3], uv: [0.0; 2], lightmap_uv: [0.0; 2], material, decal_uv: None, tangent_frame: None });
    }
    let n = r.count(4)?;
    let mut indices = Vec::with_capacity(n);
    for _ in 0..n {
        let i = r.u32()?;
        if i as usize >= vertices.len() {
            return Err("prop package: index out of range".into());
        }
        indices.push(i);
    }
    let mut extensions = Vec::new();
    for _ in 0..r.count(12)? {
        let tag: [u8; 4] = r.take(4)?.try_into().unwrap();
        extensions.push(Extension { tag, schema: r.u32()?, payload: r.bytes()? });
    }
    Ok(SkateMap {
        version,
        name,
        spawn: [0.0; 3],
        heading: 0.0,
        environment: Vec::new(),
        materials,
        textures,
        geometry: Geometry { vertices, indices, collision: Vec::new() },
        rails: Vec::new(),
        doors: Vec::new(),
        lights: Vec::new(),
        routes: Vec::new(),
        extensions,
    })
}
