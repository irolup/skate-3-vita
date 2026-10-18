//! `skater.pak` (kind `Skater`): the skinned skater and board mesh from the converted
//! `skater.glb`, ready for the GPU.
//!
//! - Joints keep the GLB skin's order, names (matched to the stock rig by name at
//!   load, as skate-game's `bind` does), parent joint and inverse bind matrix.
//! - Vertices are one interleaved buffer of [`STRIDE`] bytes (layout in [`Vertex`]),
//!   uploaded as-is. Indices are u16 over the whole buffer.
//! - Textures are each material's base colour, at most 256 × 256 with a full mip
//!   chain ([`Texture`]: DXT1, or DXT5 when the material uses alpha).
use crate::{Kind, Reader, Writer};
pub use crate::texture::{Format, Texture};

/// v2: compressed textures.
pub const VERSION: u32 = 2;
pub const STRIDE: usize = 32;
/// Byte offsets inside a vertex.
pub const POSITION: usize = 0;
pub const NORMAL: usize = 12;
pub const UV: usize = 16;
pub const JOINTS: usize = 24;
pub const WEIGHTS: usize = 28;

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    /// Nearest ancestor joint (index into the joint list), if any.
    pub parent: Option<u16>,
    /// Column-major inverse bind matrix.
    pub inverse_bind: [f32; 16],
}

/// One vertex: position (3 × f32), normal (3 × i8 normalized, 1 pad byte), uv
/// (2 × f32), joints (4 × u8, indices into the joint list), weights (4 × u8
/// normalized, summing to 255).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [i8; 3],
    pub uv: [f32; 2],
    pub joints: [u8; 4],
    pub weights: [u8; 4],
}
impl Vertex {
    pub fn encode(&self, out: &mut Vec<u8>) {
        for v in self.position {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend(self.normal.map(|v| v as u8));
        out.push(0);
        for v in self.uv {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.joints);
        out.extend_from_slice(&self.weights);
    }
    pub fn decode(b: &[u8]) -> Self {
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        Self {
            position: [f(POSITION), f(POSITION + 4), f(POSITION + 8)],
            normal: [b[NORMAL] as i8, b[NORMAL + 1] as i8, b[NORMAL + 2] as i8],
            uv: [f(UV), f(UV + 4)],
            joints: b[JOINTS..JOINTS + 4].try_into().unwrap(),
            weights: b[WEIGHTS..WEIGHTS + 4].try_into().unwrap(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub base_color: [f32; 4],
    pub texture: Option<u16>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Primitive {
    pub first_index: u32,
    pub index_count: u32,
    pub material: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skater {
    pub joints: Vec<Joint>,
    /// `STRIDE` bytes per vertex.
    pub vertex_data: Vec<u8>,
    pub indices: Vec<u16>,
    pub primitives: Vec<Primitive>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
}

impl Skater {
    pub fn vertex_count(&self) -> usize {
        self.vertex_data.len() / STRIDE
    }

    /// The sealed pack (container header + payload).
    pub fn to_pack(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.u32(self.joints.len() as u32);
        for j in &self.joints {
            w.str(&j.name);
            w.i32(j.parent.map_or(-1, i32::from));
            for v in j.inverse_bind {
                w.f32(v);
            }
        }
        w.block(&self.vertex_data);
        w.u32(self.indices.len() as u32);
        for &i in &self.indices {
            w.u16(i);
        }
        w.u32(self.primitives.len() as u32);
        for p in &self.primitives {
            w.u32(p.first_index);
            w.u32(p.index_count);
            w.u16(p.material);
        }
        w.u32(self.materials.len() as u32);
        for m in &self.materials {
            w.str(&m.name);
            for v in m.base_color {
                w.f32(v);
            }
            w.i32(m.texture.map_or(-1, i32::from));
            w.u8(match m.alpha_mode {
                AlphaMode::Opaque => 0,
                AlphaMode::Mask => 1,
                AlphaMode::Blend => 2,
            });
            w.f32(m.alpha_cutoff);
            w.u8(u8::from(m.double_sided));
        }
        w.u32(self.textures.len() as u32);
        for t in &self.textures {
            t.write(&mut w);
        }
        crate::seal(Kind::Skater, VERSION, &w.bytes)
    }

    pub fn from_pack(bytes: &[u8]) -> Result<Self, String> {
        let (kind, version, payload) = crate::open(bytes)?;
        if kind != Kind::Skater || version != VERSION {
            return Err(format!("expected a skater pack v{VERSION}, found {kind:?} v{version}"));
        }
        let mut r = Reader::new(payload);
        let joints = (0..r.u32()?)
            .map(|_| -> Result<Joint, String> {
                let name = r.str()?;
                let parent = r.i32()?;
                let mut inverse_bind = [0.0; 16];
                for v in &mut inverse_bind {
                    *v = r.f32()?;
                }
                Ok(Joint {
                    name,
                    parent: u16::try_from(parent).ok(),
                    inverse_bind,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let vertex_data = r.block()?.to_vec();
        if vertex_data.len() % STRIDE != 0 {
            return Err("vertex block is not a whole number of vertices".into());
        }
        let indices = (0..r.u32()?).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
        let primitives = (0..r.u32()?)
            .map(|_| -> Result<Primitive, String> {
                Ok(Primitive {
                    first_index: r.u32()?,
                    index_count: r.u32()?,
                    material: r.u16()?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let materials = (0..r.u32()?)
            .map(|_| -> Result<Material, String> {
                let name = r.str()?;
                let base_color = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
                let texture = u16::try_from(r.i32()?).ok();
                let alpha_mode = match r.u8()? {
                    0 => AlphaMode::Opaque,
                    1 => AlphaMode::Mask,
                    2 => AlphaMode::Blend,
                    m => return Err(format!("unknown alpha mode {m}")),
                };
                Ok(Material {
                    name,
                    base_color,
                    texture,
                    alpha_mode,
                    alpha_cutoff: r.f32()?,
                    double_sided: r.u8()? != 0,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let textures = (0..r.u32()?)
            .map(|_| Texture::read(&mut r))
            .collect::<Result<Vec<_>, _>>()?;
        r.done()?;
        let skater = Self {
            joints,
            vertex_data,
            indices,
            primitives,
            materials,
            textures,
        };
        skater.validate()?;
        Ok(skater)
    }

    /// Structural checks a renderer relies on.
    pub fn validate(&self) -> Result<(), String> {
        let vertices = self.vertex_count();
        if self.indices.iter().any(|&i| usize::from(i) >= vertices) {
            return Err("index out of the vertex range".into());
        }
        for (i, j) in self.joints.iter().enumerate() {
            if j.parent.is_some_and(|p| usize::from(p) >= i) {
                return Err(format!("joint {} has a parent after it", j.name));
            }
        }
        for chunk in self.vertex_data.chunks(STRIDE) {
            let v = Vertex::decode(chunk);
            if v.joints.iter().any(|&j| usize::from(j) >= self.joints.len()) {
                return Err("vertex joint index out of the skin".into());
            }
        }
        for p in &self.primitives {
            if (p.first_index + p.index_count) as usize > self.indices.len()
                || usize::from(p.material) >= self.materials.len()
            {
                return Err("primitive out of range".into());
            }
        }
        for m in &self.materials {
            if m.texture.is_some_and(|t| usize::from(t) >= self.textures.len()) {
                return Err(format!("material {} names a missing texture", m.name));
            }
        }
        for t in &self.textures {
            t.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Skater {
        let mut vertex_data = Vec::new();
        for i in 0..3 {
            Vertex {
                position: [i as f32, 1.5, -2.0],
                normal: [0, 127, -128],
                uv: [0.25, 0.75],
                joints: [0, 1, 0, 0],
                weights: [200, 55, 0, 0],
            }
            .encode(&mut vertex_data);
        }
        Skater {
            joints: vec![
                Joint { name: "HIPS".into(), parent: None, inverse_bind: [1.0; 16] },
                Joint { name: "SPINE".into(), parent: Some(0), inverse_bind: [0.5; 16] },
            ],
            vertex_data,
            indices: vec![0, 1, 2],
            primitives: vec![Primitive { first_index: 0, index_count: 3, material: 0 }],
            materials: vec![Material {
                name: "Hair".into(),
                base_color: [1.0, 0.9, 0.8, 1.0],
                texture: Some(0),
                alpha_mode: AlphaMode::Mask,
                alpha_cutoff: 0.5,
                double_sided: true,
            }],
            textures: vec![Texture {
                width: 2,
                height: 2,
                format: Format::Rgba8,
                mips: vec![vec![7; 16], vec![9; 4]],
            }],
        }
    }

    #[test]
    fn skater_round_trips() {
        let s = sample();
        let back = Skater::from_pack(&s.to_pack()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.vertex_count(), 3);
        assert_eq!(Vertex::decode(&back.vertex_data[STRIDE..2 * STRIDE]).position[0], 1.0);
    }

    #[test]
    fn validation_catches_broken_structure() {
        let mut s = sample();
        s.indices[2] = 9;
        assert!(Skater::from_pack(&s.to_pack()).is_err());
        let mut s = sample();
        s.textures[0].mips[1].pop();
        assert!(Skater::from_pack(&s.to_pack()).is_err());
        let mut s = sample();
        s.joints[0].parent = Some(1);
        assert!(Skater::from_pack(&s.to_pack()).is_err());
    }
}
