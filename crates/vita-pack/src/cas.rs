//! `cas.pak`: the character customisation (CAC) parts, docs/customization-survey.md step 1. A file pack (`files`) read
//! one entry at a time through its index, like `catalog.pak`:
//!
//! - `cas/library.json`: the catalog (models, materials with their retail flags such as `Cost`, `Unlock` and
//!   `SponsorUnlock`, the default outfits, colours, tattoos, morph names), each model naming its part file and each
//!   material its texture file;
//! - `cas/parts/<model id>.part` ([`Part`]): the skinned mesh on the stock rig, its second UV set (the stamps' space)
//!   and its morph targets, sparse;
//! - `cas/textures/<name>.tex` ([`texture_to_pack`]): a base colour texture, at most 256 x 256 (DXT1, or DXT5 with
//!   alpha);
//! - `cas/recipes/<name>.bin`: the disc's saved characters as they are (`skate_core::skater_record::from_retail`);
//! - `cas/rules.bin`: the catalog for the composition rules (`skate_core::cas_rules`).
use crate::skater::{Material, Primitive, Skater, Vertex, STRIDE};
use crate::texture::Texture;
use crate::{Kind, Reader, Writer};

const PART_VERSION: u32 = 1;
const TEXTURE_VERSION: u32 = 1;

/// One morph target: the vertices it moves and by how much.
#[derive(Clone, Debug, PartialEq)]
pub struct Morph {
    pub vertices: Vec<u16>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

/// A part model. `mesh` has no textures: a material of the library picks one at run time. `morphs` follow the
/// library's morph names (empty when the target moves nothing).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub mesh: Skater,
    pub uv1: Vec<[f32; 2]>,
    pub morphs: Vec<Morph>,
}

impl Part {
    pub fn to_pack(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.block(&self.mesh.to_pack());
        w.u32(self.uv1.len() as u32);
        self.uv1.iter().flatten().for_each(|&v| w.f32(v));
        w.u32(self.morphs.len() as u32);
        for m in &self.morphs {
            w.u32(m.vertices.len() as u32);
            m.vertices.iter().for_each(|&v| w.u16(v));
            m.positions.iter().chain(&m.normals).flatten().for_each(|&v| w.f32(v));
        }
        crate::seal(Kind::CasPart, PART_VERSION, &w.bytes)
    }

    pub fn from_pack(bytes: &[u8]) -> Result<Self, String> {
        let (kind, version, payload) = crate::open(bytes)?;
        if kind != Kind::CasPart || version != PART_VERSION {
            return Err(format!("expected a CAC part v{PART_VERSION}, found {kind:?} v{version}"));
        }
        let mut r = Reader::new(payload);
        let mesh = Skater::from_pack(r.block()?)?;
        let uv1 = (0..r.u32()?).map(|_| Ok([r.f32()?, r.f32()?])).collect::<Result<Vec<_>, String>>()?;
        let morphs = (0..r.u32()?)
            .map(|_| -> Result<Morph, String> {
                let n = r.u32()? as usize;
                let vertices = (0..n).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
                let mut vec3 = || (0..n).map(|_| Ok([r.f32()?, r.f32()?, r.f32()?])).collect::<Result<Vec<_>, String>>();
                let positions = vec3()?;
                let normals = vec3()?;
                Ok(Morph { vertices, positions, normals })
            })
            .collect::<Result<Vec<_>, _>>()?;
        r.done()?;
        let part = Self { mesh, uv1, morphs };
        part.validate()?;
        Ok(part)
    }

    pub fn validate(&self) -> Result<(), String> {
        let n = self.mesh.vertex_count();
        if !self.uv1.is_empty() && self.uv1.len() != n {
            return Err("CAC part: second UV set does not match the vertices".into());
        }
        if self.morphs.iter().any(|m| m.vertices.iter().any(|&v| usize::from(v) >= n)) {
            return Err("CAC part: morph vertex out of range".into());
        }
        Ok(())
    }
}

/// One worn item of an outfit: its part, its morph target weights (by `Part::morphs`, shorter means 0) and its material
/// (`texture` indexes the textures given to [`merge`]).
pub struct Piece {
    pub part: Part,
    pub weights: Vec<f32>,
    pub material: Material,
    /// Hair's opacity map (red, on the second UV set).
    pub opacity: Option<Texture>,
    /// Retail's character shading of the material.
    pub lighting: Option<Lighting>,
    /// The placement slot this material takes, the adjust index + 1 (1 shirt, 2 upper tattoo, 3 lower tattoo, 4 board,
    /// 5 hat; 0 none): its decal lies over the base colour on the second UV set.
    pub decal: u8,
}

/// The tattoos and graphics of an outfit: per material the slot it takes (see [`Piece::decal`]) and the slots' pictures.
#[derive(Clone, Debug, Default)]
pub struct Decals {
    pub target: Vec<u8>,
    pub textures: [Option<Texture>; 5],
}

/// Retail's character shading of a material (its shader family's nine `m_params` rows; upstream
/// `retail_character_common.wgsl` reads them): row 0 y the gain and z the rim falloff, row 1 the light's colour, row 2 w
/// hair's rim falloff, row 5 w the ambient light's weight, row 6 the rim colour, row 7 y the ambient normal scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    pub rows: [[f32; 4]; 9],
    /// The hair families (`character.hair`, `character.hair_ropa`) shade with their own response.
    pub hair: bool,
    /// The tint is a linear factor already (a clothes colour layer: retail's cloth shaders multiply the linear diffuse
    /// by the colour's first vector, `i_modulateColor1`), not an sRGB colour to convert.
    pub linear_tint: bool,
}

/// An outfit, ready for the renderer.
pub struct Look {
    pub skater: Skater,
    /// The second UV set, per vertex of the skater.
    pub uv1: Vec<[f32; 2]>,
    /// Hair's opacity maps, per material of the skater.
    pub opacity: Vec<Option<Texture>>,
    /// Retail's character shading, per material of the skater.
    pub lighting: Vec<Option<Lighting>>,
    /// The tattoos' targets, per material (their pictures are set by the caller: `Look::decals.textures`).
    pub decals: Decals,
}

/// An outfit as one skater mesh: every part in one vertex buffer with its morphs applied, one draw per part, on the
/// shared rig (every library part has the stock skater's joints, in order; the first part's inverse binds are kept).
pub fn merge(pieces: Vec<Piece>, textures: Vec<Texture>) -> Result<Look, String> {
    let first = pieces.first().ok_or("an outfit without parts")?;
    let mut out = Skater {
        joints: first.part.mesh.joints.clone(),
        vertex_data: Vec::new(),
        indices: Vec::new(),
        primitives: Vec::new(),
        materials: Vec::new(),
        textures,
    };
    let mut uv1 = Vec::new();
    let mut opacity = Vec::new();
    let mut lighting = Vec::new();
    let mut decal_target = Vec::new();
    for piece in pieces {
        let mesh = &piece.part.mesh;
        if !mesh.joints.iter().map(|j| &j.name).eq(out.joints.iter().map(|j| &j.name)) {
            return Err(format!("part {:?}: not on the shared rig", mesh.materials.first().map(|m| &m.name)));
        }
        let base = out.vertex_count();
        if base + mesh.vertex_count() > usize::from(u16::MAX) + 1 {
            return Err("an outfit over 65,536 vertices".into());
        }
        let mut vertices = mesh.vertex_data.clone();
        apply_morphs(&mut vertices, &piece.part.morphs, &piece.weights);
        out.vertex_data.extend_from_slice(&vertices);
        if piece.part.uv1.len() == mesh.vertex_count() {
            uv1.extend_from_slice(&piece.part.uv1);
        } else {
            uv1.resize(uv1.len() + mesh.vertex_count(), [0.0; 2]);
        }
        let material = out.materials.len() as u16;
        out.materials.push(piece.material);
        opacity.push(piece.opacity);
        lighting.push(piece.lighting);
        decal_target.push(piece.decal);
        for p in &mesh.primitives {
            let first_index = out.indices.len() as u32;
            let source = &mesh.indices[p.first_index as usize..(p.first_index + p.index_count) as usize];
            out.indices.extend(source.iter().map(|&i| i + base as u16));
            out.primitives.push(Primitive { first_index, index_count: p.index_count, material });
        }
    }
    out.validate()?;
    Ok(Look { skater: out, uv1, opacity, lighting, decals: Decals { target: decal_target, textures: [None, None, None, None, None] } })
}

/// Morph weights into the vertices (`skater::STRIDE` each); a vertex no target moves keeps its bytes.
fn apply_morphs(vertices: &mut [u8], morphs: &[Morph], weights: &[f32]) {
    let count = vertices.len() / STRIDE;
    let mut moved: Vec<Option<([f32; 3], [f32; 3])>> = vec![None; count];
    for (m, &w) in morphs.iter().zip(weights).filter(|(_, w)| **w != 0.0) {
        for ((&v, p), n) in m.vertices.iter().zip(&m.positions).zip(&m.normals) {
            let (dp, dn) = moved[usize::from(v)].get_or_insert(([0.0; 3], [0.0; 3]));
            for i in 0..3 {
                dp[i] += p[i] * w;
                dn[i] += n[i] * w;
            }
        }
    }
    for (bytes, delta) in vertices.chunks_exact_mut(STRIDE).zip(moved) {
        let Some((dp, dn)) = delta else { continue };
        let mut v = Vertex::decode(bytes);
        let mut normal = [0.0f32; 3];
        for i in 0..3 {
            v.position[i] += dp[i];
            normal[i] = f32::from(v.normal[i]) / 127.0 + dn[i];
        }
        let length = normal.iter().map(|c| c * c).sum::<f32>().sqrt();
        if length > 1e-6 {
            v.normal = normal.map(|c| (c / length * 127.0).round() as i8);
        }
        let mut encoded = Vec::with_capacity(STRIDE);
        v.encode(&mut encoded);
        bytes.copy_from_slice(&encoded);
    }
}

pub fn texture_to_pack(texture: &Texture) -> Vec<u8> {
    let mut w = Writer::default();
    texture.write(&mut w);
    crate::seal(Kind::CasTexture, TEXTURE_VERSION, &w.bytes)
}

pub fn texture_from_pack(bytes: &[u8]) -> Result<Texture, String> {
    let (kind, version, payload) = crate::open(bytes)?;
    if kind != Kind::CasTexture || version != TEXTURE_VERSION {
        return Err(format!("expected a CAC texture v{TEXTURE_VERSION}, found {kind:?} v{version}"));
    }
    let mut r = Reader::new(payload);
    let texture = Texture::read(&mut r)?;
    r.done()?;
    texture.validate()?;
    Ok(texture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skater::{AlphaMode, Joint};
    use crate::texture::Format;

    fn mesh() -> Skater {
        let mut vertex_data = Vec::new();
        for i in 0..3 {
            Vertex { position: [i as f32, 0.0, 0.0], normal: [0, 127, 0], uv: [0.0, 1.0], joints: [0; 4], weights: [255, 0, 0, 0] }
                .encode(&mut vertex_data);
        }
        assert_eq!(vertex_data.len(), 3 * STRIDE);
        let identity: [f32; 16] = std::array::from_fn(|i| f32::from(u8::from(i % 5 == 0)));
        Skater {
            joints: vec![Joint { name: "root".into(), parent: None, inverse_bind: identity }],
            vertex_data,
            indices: vec![0, 1, 2],
            primitives: vec![Primitive { first_index: 0, index_count: 3, material: 0 }],
            materials: vec![Material {
                name: "part".into(),
                base_color: [1.0; 4],
                texture: None,
                alpha_mode: AlphaMode::Opaque,
                alpha_cutoff: 0.5,
                double_sided: false,
            }],
            textures: Vec::new(),
        }
    }

    #[test]
    fn merge_offsets_the_parts_and_applies_morphs() {
        let material = |name: &str, texture| Material {
            name: name.into(),
            base_color: [0.5, 0.5, 0.5, 1.0],
            texture: Some(texture),
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        };
        let morph = Morph { vertices: vec![1], positions: vec![[0.0, 2.0, 0.0]], normals: vec![[1.0, 0.0, 0.0]] };
        let untouched = mesh();
        let texture = Texture { width: 1, height: 1, format: Format::Rgba8, mips: vec![vec![1; 4]] };
        let pieces = vec![
            Piece { part: Part { mesh: mesh(), uv1: Vec::new(), morphs: vec![morph.clone()] }, weights: vec![0.5], material: material("a", 0), opacity: None, lighting: None, decal: 0 },
            Piece { part: Part { mesh: mesh(), uv1: vec![[0.5, 0.25]; 3], morphs: vec![morph] }, weights: vec![0.0], material: material("b", 1), opacity: Some(texture.clone()), lighting: None, decal: 0 },
        ];
        let look = merge(pieces, vec![texture.clone(), texture]).unwrap();
        assert_eq!((look.uv1.len(), look.uv1[0], look.uv1[3]), (6, [0.0; 2], [0.5, 0.25]));
        assert_eq!((look.opacity[0].is_none(), look.opacity[1].is_some()), (true, true));
        let out = look.skater;
        assert_eq!(out.vertex_count(), 6);
        assert_eq!(out.indices, [0, 1, 2, 3, 4, 5]);
        assert_eq!(out.primitives[1], Primitive { first_index: 3, index_count: 3, material: 1 });
        assert_eq!((out.materials[1].name.as_str(), out.materials.len()), ("b", 2));
        let v = Vertex::decode(&out.vertex_data[STRIDE..2 * STRIDE]);
        assert_eq!(v.position, [1.0, 1.0, 0.0]);
        // Normal (0, 1, 0) + (0.5, 0, 0), renormalized.
        assert_eq!(v.normal, [57, 114, 0]);
        // Weight 0 and vertices no target moves stay as they were.
        assert_eq!(out.vertex_data[4 * STRIDE..5 * STRIDE], untouched.vertex_data[STRIDE..2 * STRIDE]);
        assert_eq!(out.vertex_data[..STRIDE], untouched.vertex_data[..STRIDE]);
        assert!(merge(Vec::new(), Vec::new()).is_err());
    }

    #[test]
    fn part_and_texture_round_trip() {
        let part = Part {
            mesh: mesh(),
            uv1: vec![[0.25, 0.5]; 3],
            morphs: vec![
                Morph { vertices: vec![2], positions: vec![[0.0, 0.01, 0.0]], normals: vec![[0.0, 0.0, 0.1]] },
                Morph { vertices: Vec::new(), positions: Vec::new(), normals: Vec::new() },
            ],
        };
        assert_eq!(Part::from_pack(&part.to_pack()).unwrap(), part);
        let bad = Part { morphs: vec![Morph { vertices: vec![3], positions: vec![[0.0; 3]], normals: vec![[0.0; 3]] }], ..part };
        assert!(Part::from_pack(&bad.to_pack()).is_err());
        let texture = Texture { width: 4, height: 4, format: Format::Rgba8, mips: vec![vec![1; 64], vec![2; 16], vec![3; 4]] };
        assert_eq!(texture_from_pack(&texture_to_pack(&texture)).unwrap(), texture);
    }
}
