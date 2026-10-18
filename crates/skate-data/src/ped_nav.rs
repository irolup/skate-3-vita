//! Pedestrian navmesh data (doc 26, peds milestone M3): `private/living_world/navmesh.bin`, the
//! retail NavPower nav graphs (`0x00EB0027`) that setup decodes and joins across tiles
//! (`tools/asset_pipeline/living_world_navmesh.py` documents both formats), read into
//! `skate_core::living_world::peds::NavMeshInput`.
//!
//! `navmesh.bin` (little-endian): `LWNAVMSH`, u32 version 1, u32 district count; per district:
//! u8 name length + ASCII, f32 x4 agent parameters, u32 vertex count + f32 x3 each, u32 polygon
//! count, per polygon: u32 first vertex, u16 vertex count, u8 area, u8 tile-stitched edge count,
//! u32 flags, then i32 neighbour per edge (-1 none). A mod map writes the same file with
//! [`write`] (or fills `NavMeshInput` directly).

use skate_core::living_world::peds::{NavMeshInput, NavPolyInput};

pub const MAGIC: &[u8; 8] = b"LWNAVMSH";
pub const VERSION: u32 = 1;
/// The file under an asset root.
pub const NAVMESH: &str = "private/living_world/navmesh.bin";

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self.b.get(self.at..self.at + n).ok_or_else(|| format!("navmesh.bin truncated at {}", self.at))?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
}

/// Every district's navmesh, by district name (sorted).
pub fn parse(bytes: &[u8]) -> Result<Vec<(String, NavMeshInput)>, String> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err("not a navmesh.bin".into());
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(format!("navmesh.bin version {version}, expected {VERSION}"));
    }
    let count = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..count {
        let n = r.u8()? as usize;
        let name = String::from_utf8_lossy(r.take(n)?).into_owned();
        let agent = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let nv = r.u32()? as usize;
        let mut verts = Vec::with_capacity(nv.min(1 << 24));
        for _ in 0..nv {
            verts.push([r.f32()?, r.f32()?, r.f32()?]);
        }
        let np = r.u32()? as usize;
        let mut polygons = Vec::with_capacity(np.min(1 << 22));
        for _ in 0..np {
            let first = r.u32()? as usize;
            let vc = r.u16()? as usize;
            let area = r.u8()?;
            let _stitched = r.u8()?;
            let _flags = r.u32()?;
            let mut neighbours = Vec::with_capacity(vc);
            for _ in 0..vc {
                let q = r.u32()? as i32;
                neighbours.push(if q >= 0 && (q as usize) < np { Some(q as u32) } else { None });
            }
            let poly_verts = verts.get(first..first + vc).ok_or_else(|| format!("{name}: polygon vertices out of range"))?.to_vec();
            polygons.push(NavPolyInput { verts: poly_verts, neighbours, area });
        }
        out.push((name, NavMeshInput { agent, polygons }));
    }
    Ok(out)
}

/// One district's navmesh (`None` when the file has none for it).
pub fn district(bytes: &[u8], name: &str) -> Result<Option<NavMeshInput>, String> {
    Ok(parse(bytes)?.into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, m)| m))
}

/// Write districts in the same format (flags 0, no stitch counts).
pub fn write(districts: &[(String, NavMeshInput)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend(VERSION.to_le_bytes());
    out.extend((districts.len() as u32).to_le_bytes());
    for (name, m) in districts {
        out.push(name.len() as u8);
        out.extend(name.as_bytes());
        for a in m.agent {
            out.extend(a.to_le_bytes());
        }
        let nv: usize = m.polygons.iter().map(|p| p.verts.len()).sum();
        out.extend((nv as u32).to_le_bytes());
        for v in m.polygons.iter().flat_map(|p| p.verts.iter()) {
            for c in v {
                out.extend(c.to_le_bytes());
            }
        }
        out.extend((m.polygons.len() as u32).to_le_bytes());
        let mut first = 0u32;
        for p in &m.polygons {
            out.extend(first.to_le_bytes());
            out.extend((p.verts.len() as u16).to_le_bytes());
            out.push(p.area);
            out.push(0);
            out.extend(0u32.to_le_bytes());
            for k in 0..p.verts.len() {
                let q = p.neighbours.get(k).copied().flatten().map_or(-1i32, |q| q as i32);
                out.extend(q.to_le_bytes());
            }
            first += p.verts.len() as u32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejects() {
        let tri = |x: f32, n: Option<u32>| NavPolyInput { verts: vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 0.0, 1.0]], neighbours: vec![n, None, None], area: 0x11 };
        let mesh = NavMeshInput { agent: [0.12, 0.35, 0.2, 1.6], polygons: vec![tri(0.0, Some(1)), tri(1.0, Some(0))] };
        let bytes = write(&[("DownTown".into(), mesh.clone())]);
        let back = parse(&bytes).unwrap();
        assert_eq!(back, vec![("DownTown".to_string(), mesh.clone())]);
        assert_eq!(district(&bytes, "downtown").unwrap(), Some(mesh));
        assert_eq!(district(&bytes, "Industrial").unwrap(), None);
        assert!(parse(b"LWNAVMSH").is_err());
        assert!(parse(&bytes[..bytes.len() - 3]).is_err());
        assert!(parse(b"NOTAMESHxxxxxxxx").is_err());
    }
}
