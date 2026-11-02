//! The Object Dropper catalogue (`catalog.pak`, docs/object-dropper-survey.md step D2): each item's render read on
//! its own when the editor needs it, its textures from a table shared by the whole catalogue part (`terrain` or
//! `props`), so an item that shares a texture with one already loaded does not load it again.
//!
//! `ItemRender`: the item's draws over its own vertex and index data; a draw's `albedo` / `lightmap` is an index in
//! its part's texture table (`CatalogTexture` files), not in a texture list of its own.
use crate::map::{Draw, STRIDE};
use crate::texture::Texture;
use crate::{Kind, Reader, Writer};

const ITEM_VERSION: u32 = 1;
const TEXTURE_VERSION: u32 = 1;

#[derive(Debug, PartialEq)]
pub struct ItemRender {
    pub vertex_data: Vec<u8>,
    pub indices: Vec<u16>,
    pub draws: Vec<Draw>,
}

impl ItemRender {
    pub fn to_pack(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.block(&self.vertex_data);
        w.u32(self.indices.len() as u32);
        self.indices.iter().for_each(|&i| w.u16(i));
        w.u32(self.draws.len() as u32);
        for d in &self.draws {
            for v in [d.first_vertex, d.vertex_count, d.first_index, d.index_count] {
                w.u32(v);
            }
            for t in [d.albedo, d.lightmap] {
                w.i32(t.map_or(-1, i32::from));
            }
            d.color.iter().for_each(|&v| w.f32(v));
            w.f32(d.alpha_cutoff.unwrap_or(-1.0));
            d.bounds_min.iter().chain(&d.bounds_max).for_each(|&v| w.f32(v));
        }
        crate::seal(Kind::CatalogItem, ITEM_VERSION, &w.bytes)
    }

    /// `textures`: the size of the item's part's texture table, which every draw's textures must index.
    pub fn from_pack(bytes: &[u8], textures: usize) -> Result<Self, String> {
        let (kind, version, payload) = crate::open(bytes)?;
        if kind != Kind::CatalogItem || version != ITEM_VERSION {
            return Err(format!("expected a catalogue item v{ITEM_VERSION}, found {kind:?} v{version}"));
        }
        let mut r = Reader::new(payload);
        let vertex_data = r.block()?.to_vec();
        let indices = (0..r.u32()?).map(|_| r.u16()).collect::<Result<Vec<_>, _>>()?;
        let draws = (0..r.u32()?)
            .map(|_| -> Result<Draw, String> {
                let (first_vertex, vertex_count, first_index, index_count) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let albedo = u16::try_from(r.i32()?).ok();
                let lightmap = u16::try_from(r.i32()?).ok();
                let color = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
                let cutoff = r.f32()?;
                Ok(Draw {
                    first_vertex,
                    vertex_count,
                    first_index,
                    index_count,
                    albedo,
                    lightmap,
                    color,
                    alpha_cutoff: (cutoff >= 0.0).then_some(cutoff),
                    bounds_min: [r.f32()?, r.f32()?, r.f32()?],
                    bounds_max: [r.f32()?, r.f32()?, r.f32()?],
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        r.done()?;
        let item = Self { vertex_data, indices, draws };
        item.validate(textures)?;
        Ok(item)
    }

    fn validate(&self, textures: usize) -> Result<(), String> {
        if self.vertex_data.len() % STRIDE != 0 {
            return Err("catalogue item vertex block is not a whole number of vertices".into());
        }
        let vertices = self.vertex_data.len() / STRIDE;
        for d in &self.draws {
            if (d.first_vertex + d.vertex_count) as usize > vertices
                || (d.first_index + d.index_count) as usize > self.indices.len()
                || self.indices[d.first_index as usize..(d.first_index + d.index_count) as usize].iter().any(|&i| u32::from(i) >= d.vertex_count)
                || [d.albedo, d.lightmap].iter().flatten().any(|&t| usize::from(t) >= textures)
            {
                return Err("catalogue item draw out of range".into());
            }
        }
        Ok(())
    }
}

/// One texture of a catalogue part's shared table.
pub fn texture_to_pack(texture: &Texture) -> Vec<u8> {
    let mut w = Writer::default();
    texture.write(&mut w);
    crate::seal(Kind::CatalogTexture, TEXTURE_VERSION, &w.bytes)
}

pub fn texture_from_pack(bytes: &[u8]) -> Result<Texture, String> {
    let (kind, version, payload) = crate::open(bytes)?;
    if kind != Kind::CatalogTexture || version != TEXTURE_VERSION {
        return Err(format!("expected a catalogue texture v{TEXTURE_VERSION}, found {kind:?} v{version}"));
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
    use crate::texture::Format;

    #[test]
    fn item_and_texture_round_trip() {
        let draw = Draw {
            first_vertex: 0,
            vertex_count: 3,
            first_index: 0,
            index_count: 3,
            albedo: Some(4),
            lightmap: None,
            color: [1.0, 0.5, 0.25, 1.0],
            alpha_cutoff: Some(0.5),
            bounds_min: [-1.0; 3],
            bounds_max: [1.0; 3],
        };
        let item = ItemRender { vertex_data: vec![7; 3 * STRIDE], indices: vec![0, 1, 2], draws: vec![draw] };
        let bytes = item.to_pack();
        assert_eq!(ItemRender::from_pack(&bytes, 5).unwrap(), item);
        // The draw's texture is outside a table of 4.
        assert!(ItemRender::from_pack(&bytes, 4).is_err());
        let texture = Texture { width: 4, height: 4, format: Format::Rgba8, mips: vec![vec![1; 64], vec![2; 16], vec![3; 4]] };
        assert_eq!(texture_from_pack(&texture_to_pack(&texture)).unwrap(), texture);
    }
}
