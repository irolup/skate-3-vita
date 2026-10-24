//! skate_vita (Phase 2): a skate.Park layout baked for the Vita. Upstream fuses a layout's pieces when the map loads
//! (`skate_world::append_pieces`, 89-345 ms a Fullpark on a desktop, several seconds on the Vita) and reads their
//! rails from the layout's sidecar (up to 14 MB of JSON). `vita-bake` runs the same code once and stores the result as
//! an extension on the park's map ([`EXTENSION_TAG`]): the fused rows ([`crate::skate_world::FusedTriangle`], the
//! kept triangles with their edge features, one range per piece) and, for the rails, each piece's id, template and
//! placement with the templates' splines. At load the sim builds the same triangles (`skate_world::parts_from_rows`)
//! and poses the rails (`park_layout::pose_rails`), so the world is the one upstream's fuser builds.
use skate_data::binary::{Reader, Writer};
use skate_data::skate_map::{Extension, SkateMap};
use std::path::Path;

/// The extension a baked park map carries.
pub const EXTENSION_TAG: [u8; 4] = *b"PKFU";
const KIND: &[u8; 4] = b"PKF1";

/// What a baked layout gives the sim: the fused rows (read in place from the map's extension, [`ROW`] bytes each: a
/// copy was 9 MB more of heap for MegaPark while its world was built) with one range per piece, and the posed rails.
pub(crate) struct Baked<'a> {
    rows: &'a [u8],
    pub ranges: Vec<std::ops::Range<usize>>,
    pub rails: Vec<crate::park_layout::PieceRails>,
}

/// A stored row: nine coordinates, surface, packed surface, flags, three cosines (u32 each).
const ROW: usize = 15 * 4;

impl Baked<'_> {
    pub(crate) fn len(&self) -> usize {
        self.rows.len() / ROW
    }

    pub(crate) fn rows(&self) -> impl ExactSizeIterator<Item = crate::skate_world::FusedTriangle> + '_ {
        self.rows.chunks_exact(ROW).map(|row| {
            let u = |i: usize| u32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap());
            let f = |i: usize| f32::from_bits(u(i));
            crate::skate_world::FusedTriangle {
                points: [[f(0), f(1), f(2)], [f(3), f(4), f(5)], [f(6), f(7), f(8)]],
                surface: u(9),
                packed: u(10) as u16,
                flags: u(11),
                cosines: [f(12), f(13), f(14)],
            }
        })
    }
}

/// A map marked with `layout` (what `park_layout::attach` writes), to read the layout through `park_layout`.
fn marked(layout: &str) -> SkateMap {
    SkateMap {
        version: 0,
        name: String::new(),
        spawn: [0.0; 3],
        heading: 0.0,
        environment: Vec::new(),
        materials: Vec::new(),
        textures: Vec::new(),
        geometry: skate_data::skate_map::Geometry { vertices: Vec::new(), indices: Vec::new(), collision: Vec::new() },
        rails: Vec::new(),
        doors: Vec::new(),
        lights: Vec::new(),
        routes: Vec::new(),
        extensions: vec![Extension { tag: crate::park_layout::EXTENSION_TAG, schema: 1, payload: layout.as_bytes().to_vec() }],
    }
}

/// Counts of a bake, for its log: fused triangles, pieces, railed pieces.
pub struct BakeStats {
    pub triangles: usize,
    pub pieces: usize,
    pub railed: usize,
}

/// Bakes `layout` (`private/park-layouts/<layout>.*` under `assets`) into the extension a park map carries.
pub fn bake(assets: &Path, layout: &str) -> Result<(Extension, BakeStats), String> {
    let map = marked(layout);
    let pieces = crate::park_layout::load_pieces(assets, &map).ok_or_else(|| format!("{layout}: no pieces"))?;
    let (rows, ranges) = crate::skate_world::fused_piece_rows(&pieces)?;
    let (_, objects) = crate::park_layout::load_package(assets, &map).ok_or_else(|| format!("{layout}: no package"))?;
    // The rails' sources: each piece's placement and the splines of the templates that have rails.
    let sidecar = crate::park_layout::directory(assets).join(format!("{layout}.json"));
    let value: serde_json::Value = serde_json::from_slice(&skate_data::vfs::read(&sidecar).map_err(|e| format!("{}: {e}", sidecar.display()))?)
        .map_err(|e| format!("{}: {e}", sidecar.display()))?;
    let splines = value.get("grind_splines").and_then(|v| v.as_object()).cloned().unwrap_or_default();
    let mut used = serde_json::Map::new();
    let mut placed = Vec::new();
    for object in &objects {
        let template = object.name.split('/').next().unwrap_or_default();
        if let Some(list) = splines.get(template) {
            used.insert(template.to_owned(), list.clone());
            placed.push(serde_json::json!({ "id": object.id, "template": template, "transform": object.transform }));
        }
    }
    let rails = serde_json::to_string(&serde_json::json!({ "grind_splines": used, "pieces": placed })).map_err(|e| e.to_string())?;
    let mut w = Writer::new(KIND);
    w.str(&rails);
    w.u32(ranges.len() as u32);
    for r in &ranges {
        w.u32(r.start as u32);
        w.u32(r.end as u32);
    }
    w.u32(rows.len() as u32);
    for row in &rows {
        for v in row.points.iter().flatten() {
            w.u32(v.to_bits());
        }
        w.u32(row.surface);
        w.u32(u32::from(row.packed));
        w.u32(row.flags);
        for c in row.cosines {
            w.u32(c.to_bits());
        }
    }
    let stats = BakeStats { triangles: rows.len(), pieces: ranges.len(), railed: placed.len() };
    Ok((Extension { tag: EXTENSION_TAG, schema: 1, payload: w.bytes }, stats))
}

/// The baked layout a loaded map carries, if any.
pub(crate) fn decode(map: &SkateMap) -> Option<Result<Baked<'_>, String>> {
    let extension = map.extensions.iter().find(|e| e.tag == EXTENSION_TAG && e.schema == 1)?;
    Some(read(&extension.payload))
}

fn read(bytes: &[u8]) -> Result<Baked<'_>, String> {
    let mut r = Reader::new(bytes, KIND)?;
    let rails: serde_json::Value = serde_json::from_str(&r.str()?).map_err(|e| format!("baked park rails: {e}"))?;
    let ranges = (0..r.u32()?).map(|_| Ok(r.u32()? as usize..r.u32()? as usize)).collect::<Result<Vec<_>, String>>()?;
    let count = r.u32()? as usize;
    let rows = r.bytes(count.checked_mul(ROW).ok_or("baked park: row count")?)?;
    r.done()?;
    if ranges.iter().any(|range| range.start > range.end || range.end > count) {
        return Err("baked park: a piece range outside the rows".into());
    }
    let mut templates = std::collections::BTreeMap::new();
    for (key, list) in rails["grind_splines"].as_object().into_iter().flatten() {
        templates.insert(key.clone(), crate::grind_world::props::parse_rails(list).map_err(|e| format!("baked park rails {key}: {e}"))?);
    }
    let mut posed = Vec::new();
    for piece in rails["pieces"].as_array().into_iter().flatten() {
        let (Some(id), Some(template)) = (piece["id"].as_u64(), piece["template"].as_str()) else { continue };
        let values: Vec<f32> = piece["transform"].as_array().into_iter().flatten().filter_map(|v| v.as_f64().map(|v| v as f32)).collect();
        let (Ok(transform), Some(list)) = (<[f32; 12]>::try_from(values), templates.get(template)) else { continue };
        posed.push(crate::park_layout::PieceRails { piece: id as u32, segments: crate::park_layout::pose_rails(id as u32, &transform, list) });
    }
    Ok(Baked { rows, ranges, rails: posed })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Fullpark baked and read back gives the world upstream's fuser builds at load: the same triangles, packed
    /// surfaces and query meshes, and the same rails. Data-gated (`SKATE3_ASSET_ROOT`).
    #[test]
    #[ignore = "needs the private install data"]
    fn a_baked_layout_builds_the_fused_world() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let material = skate_core::physics::contact::RetailContactMaterial { static_friction: 0., dynamic_friction: 0., restitution: 1. };
        for layout in ["Downtown_Fullpark", "MegaPark_Fullpark"] {
            let map = marked(layout);
            let pieces = crate::park_layout::load_pieces(&assets, &map).unwrap();
            let (extension, stats) = bake(&assets, layout).unwrap();
            let mut baked_map = marked("none");
            baked_map.extensions = vec![extension];
            let baked = decode(&baked_map).unwrap().unwrap();
            let empty = marked("none");
            let fused = crate::skate_world::collision_world_with(&empty, material, Some(crate::skate_world::LayoutPieces::Loaded(&pieces))).unwrap();
            let read = crate::skate_world::collision_world_with(&empty, material, Some(crate::skate_world::LayoutPieces::Baked(&baked))).unwrap();
            assert_eq!(fused.triangles().len(), stats.triangles);
            // Bit for bit (Debug prints every field; NaN-free here).
            let text = |w: &skate_core::physics::board_world::BoardWorld| w.triangles().iter().map(|t| format!("{t:?}")).collect::<Vec<_>>();
            assert!(text(&fused) == text(&read), "{layout}: the same triangles");
            assert_eq!(pieces.rails.len(), baked.rails.len(), "{layout}: railed pieces");
            let segments = |r: &[crate::park_layout::PieceRails]| r.iter().map(|p| p.segments.len()).sum::<usize>();
            assert_eq!(segments(&pieces.rails), segments(&baked.rails));
            println!("BAKED {layout}: {} triangles, {} pieces, {} railed, {} KiB", stats.triangles, stats.pieces, stats.railed, baked_map.extensions[0].payload.len() / 1024);
        }
    }
}
