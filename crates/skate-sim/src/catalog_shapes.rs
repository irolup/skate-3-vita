//! skate_vita: the Object Dropper catalogue's collision shapes in binary (docs/object-dropper-survey.md step D2).
//! The export's sidecars carry them as JSON (`collision`: per template its triangles, edge bytes, surfaces and fuse
//! tags, read by `skate_world::collision_shapes`); as text they were 61 of the catalogue pack's 78 MB, and a 1 MB
//! JSON parse is slow on the Vita. `vita-bake catalog` writes each template's shape with [`encode`] and the game
//! reads it back with [`decode`] into the same `PropShape`.
//!
//! Layout (little-endian): `SHP1`, u32 template count; per template: u16 name length, name, u32 triangle count,
//! u8 flags (bit 0: fuse tags present); per triangle: 9 f32 points, u8 edge flag (0: none) and 3 edge bytes,
//! u32 surface, then with fuse tags: u8 (bit 0: keys present, bit 1: second half) and 3 u64 keys.
use std::collections::BTreeMap;

use crate::skate_world::{FuseTag, PropShape};

const MAGIC: &[u8; 4] = b"SHP1";

/// The binary shapes of a sidecar's `collision` map (only the templates whose shape parses, as the JSON loader keeps).
pub fn encode(sidecar: &serde_json::Value, origin: &str) -> Vec<u8> {
    let shapes = crate::skate_world::collision_shapes(sidecar, origin);
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(shapes.len() as u32).to_le_bytes());
    for (name, shape) in &shapes {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(shape.triangles.len() as u32).to_le_bytes());
        let fuse = shape.fuse.len() == shape.triangles.len();
        out.push(u8::from(fuse));
        for i in 0..shape.triangles.len() {
            for p in shape.triangles[i].iter().flatten() {
                out.extend_from_slice(&p.to_le_bytes());
            }
            match shape.edges[i] {
                Some(e) => out.extend_from_slice(&[1, e[0], e[1], e[2]]),
                None => out.extend_from_slice(&[0; 4]),
            }
            out.extend_from_slice(&shape.surfaces[i].to_le_bytes());
            if fuse {
                let tag = shape.fuse[i];
                out.push(u8::from(tag.keys.is_some()) | u8::from(tag.second_half) << 1);
                for k in tag.keys.unwrap_or_default() {
                    out.extend_from_slice(&k.to_le_bytes());
                }
            }
        }
    }
    out
}

pub(crate) fn decode(bytes: &[u8]) -> Result<BTreeMap<String, PropShape>, String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let slice = bytes.get(at..at + n).ok_or("catalogue shapes: truncated")?;
        at += n;
        Ok(slice)
    };
    if take(4)? != MAGIC {
        return Err("catalogue shapes: bad magic".into());
    }
    let u32_of = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
    let count = u32_of(take(4)?);
    let mut out = BTreeMap::new();
    for _ in 0..count {
        let name_len = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        let name = std::str::from_utf8(take(name_len)?).map_err(|e| e.to_string())?.to_owned();
        let triangles = u32_of(take(4)?) as usize;
        let fuse = take(1)?[0] & 1 != 0;
        let mut shape = PropShape::default();
        for _ in 0..triangles {
            let raw = take(36)?;
            let f = |i: usize| f32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap());
            shape.triangles.push([[f(0), f(1), f(2)], [f(3), f(4), f(5)], [f(6), f(7), f(8)]]);
            let edge = take(4)?;
            shape.edges.push((edge[0] != 0).then_some([edge[1], edge[2], edge[3]]));
            shape.surfaces.push(u32_of(take(4)?));
            if fuse {
                let flags = take(1)?[0];
                let keys = take(24)?;
                let k = |i: usize| u64::from_le_bytes(keys[i * 8..i * 8 + 8].try_into().unwrap());
                shape.fuse.push(FuseTag { keys: (flags & 1 != 0).then(|| [k(0), k(1), k(2)]), second_half: flags & 2 != 0 });
            }
        }
        out.insert(name, shape);
    }
    if at != bytes.len() {
        return Err("catalogue shapes: trailing bytes".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn shapes_round_trip_through_binary() {
        let sidecar = serde_json::json!({ "collision": {
            "AAAA": {
                "triangles": [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.5, -2.25]], [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]]],
                "edges": [[1, 2, 3], [0, 0, 255]],
                "surfaces": [7, 9],
                "units": [[2, 1], [1, 0]],
                "vertex_keys": [[[1, 2, 3], [0, 0, 1], [5, 6, 7]], [[1, 1, 1], [2, 2, 2], [3, 3, 3]]],
            },
            "BBBB": { "triangles": [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]], "edges": [[0, 0, 0]], "surfaces": [1] },
        }});
        let json = crate::skate_world::collision_shapes(&sidecar, "test");
        let binary = super::decode(&super::encode(&sidecar, "test")).unwrap();
        assert_eq!(json.keys().collect::<Vec<_>>(), binary.keys().collect::<Vec<_>>());
        for (name, a) in &json {
            let b = &binary[name];
            assert_eq!(a.triangles, b.triangles);
            assert_eq!(a.edges, b.edges);
            assert_eq!(a.surfaces, b.surfaces);
            assert_eq!(a.fuse, b.fuse);
        }
        assert_eq!(binary["AAAA"].fuse.len(), 2);
        assert!(binary["AAAA"].fuse[0].second_half);
        assert!(binary["BBBB"].fuse.is_empty());
    }
}
