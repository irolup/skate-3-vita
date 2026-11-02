//! skate_vita (skate.Park saving, docs/skatepark-survey.md step S2): retail's `.park` layout file, read and written.
//!
//! Layout (big-endian; upstream `park_layout.parse`, research b425): version, four block lengths, the origin world's
//! hash (the template world the layout started from: `dist_downtownpark_full`, `dist_startpark_empty`, ...), then the
//! blocks: the static pieces (format 1: position, rotation x y z w, recipe id, 36 bytes; format 2 adds a tint rgb, 48
//! bytes), the movable props (position, rotation, recipe id, a flag, 40 bytes), the distinct static recipe ids and the
//! distinct prop recipe ids (a count, then the ids in first-use order), and the editor tail whose first matrix is the
//! player start (rows right, up, forward, translation; the w lanes are not read). Retail's piece and prop blocks run on
//! past their records (its buffers' capacity: stale records and fill bytes); they are not read and not written.
//!
//! NOT RETAIL YET: the tail after the start is kept as read and written back as it is (its meaning not traced).
use glam::{Quat, Vec3};

/// A layout piece: its pose, recipe id (the catalogue item, as `%016X`) and tint (rgb, white untinted).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub position: Vec3,
    pub rotation: Quat,
    pub item: u64,
    pub tint: [f32; 3],
}

/// A movable prop: its pose, recipe id and the record's flag.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub position: Vec3,
    pub rotation: Quat,
    pub item: u64,
    pub flag: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParkFile {
    pub version: u32,
    /// The template world's key hash (`environment::key_hash` of `dist_<lot>_<template>`).
    pub origin: u64,
    pub pieces: Vec<Piece>,
    pub props: Vec<Prop>,
    /// The player start (rows right, up, forward, translation), None when the tail has none.
    pub start: Option<[[f32; 4]; 4]>,
    /// The tail after the start, as read.
    pub tail: Vec<u8>,
}

const STATIC_TINTED: u32 = 2;
const STATIC_STRIDE: [usize; 3] = [0, 36, 48];
const PROP_STRIDE: usize = 40;
/// The written version (the newest on the disc: Industrial's and MegaPark's Fullparks).
pub const VERSION: u32 = 12;

fn u32_at(raw: &[u8], at: usize) -> Result<u32, String> {
    raw.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).ok_or_else(|| "truncated park layout".to_owned())
}

fn f32_at(raw: &[u8], at: usize) -> Result<f32, String> {
    u32_at(raw, at).map(f32::from_bits)
}

fn u64_at(raw: &[u8], at: usize) -> Result<u64, String> {
    raw.get(at..at + 8).map(|b| u64::from_be_bytes(b.try_into().unwrap())).ok_or_else(|| "truncated park layout".to_owned())
}

/// A pose at `at`: position, then the rotation as x, y, z, w.
fn pose_at(raw: &[u8], at: usize) -> Result<(Vec3, Quat), String> {
    let f = |k: usize| f32_at(raw, at + k * 4);
    Ok((Vec3::new(f(0)?, f(1)?, f(2)?), Quat::from_xyzw(f(3)?, f(4)?, f(5)?, f(6)?)))
}

impl ParkFile {
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        let version = u32_at(raw, 0)?;
        let lengths = [u32_at(raw, 4)?, u32_at(raw, 8)?, u32_at(raw, 12)?, u32_at(raw, 16)?].map(|l| l as usize);
        let origin = u64_at(raw, 20)?;
        let mut starts = [28usize; 5];
        for k in 0..4 {
            starts[k + 1] = starts[k] + lengths[k];
        }
        if starts[4] > raw.len() {
            return Err("park layout blocks run past the file".into());
        }
        let mut pieces = Vec::new();
        if lengths[0] != 0 {
            let (format, count) = (u32_at(raw, starts[0])?, u32_at(raw, starts[0] + 4)? as usize);
            let stride = *STATIC_STRIDE.get(format as usize).filter(|s| **s != 0).ok_or(format!("park static block format {format}"))?;
            if 8 + count * stride > lengths[0] {
                return Err(format!("park static block: {count} pieces do not fit"));
            }
            for i in 0..count {
                let at = starts[0] + 8 + i * stride;
                let (position, rotation) = pose_at(raw, at)?;
                let tint = if format == STATIC_TINTED { [f32_at(raw, at + 36)?, f32_at(raw, at + 40)?, f32_at(raw, at + 44)?] } else { [1.0; 3] };
                pieces.push(Piece { position, rotation, item: u64_at(raw, at + 28)?, tint });
            }
        }
        let mut props = Vec::new();
        if lengths[1] != 0 {
            let (format, count) = (u32_at(raw, starts[1])?, u32_at(raw, starts[1] + 4)? as usize);
            if format != 1 || 8 + count * PROP_STRIDE > lengths[1] {
                return Err(format!("park prop block format {format}, {count} props"));
            }
            for i in 0..count {
                let at = starts[1] + 8 + i * PROP_STRIDE;
                let (position, rotation) = pose_at(raw, at)?;
                props.push(Prop { position, rotation, item: u64_at(raw, at + 28)?, flag: u32_at(raw, at + 36)? });
            }
        }
        // The tail's first matrix, when it is one (three unit rows, finite).
        let tail_at = starts[4];
        let mut start = None;
        if tail_at + 64 <= raw.len() {
            let m: [[f32; 4]; 4] = std::array::from_fn(|r| std::array::from_fn(|c| f32_at(raw, tail_at + (r * 4 + c) * 4).unwrap_or(f32::NAN)));
            let unit = |r: usize| (m[r][0] * m[r][0] + m[r][1] * m[r][1] + m[r][2] * m[r][2] - 1.0).abs() < 0.01;
            if m.iter().all(|row| row[..3].iter().all(|v| v.is_finite())) && (0..3).all(unit) {
                start = Some([[m[0][0], m[0][1], m[0][2], 0.0], [m[1][0], m[1][1], m[1][2], 0.0], [m[2][0], m[2][1], m[2][2], 0.0], [m[3][0], m[3][1], m[3][2], 1.0]]);
            }
        }
        let tail = raw[tail_at + if start.is_some() { 64 } else { 0 }..].to_vec();
        Ok(Self { version, origin, pieces, props, start, tail })
    }

    /// The distinct recipe ids of `ids`, in first-use order (the third and fourth blocks).
    fn distinct(ids: impl Iterator<Item = u64>) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::new();
        for id in ids {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }

    /// The third and fourth blocks as written: the pieces' and the props' distinct recipe ids.
    pub fn id_lists(&self) -> [Vec<u8>; 2] {
        let list = |ids: Vec<u64>| {
            let mut out = (ids.len() as u32).to_be_bytes().to_vec();
            for id in ids {
                out.extend_from_slice(&id.to_be_bytes());
            }
            out
        };
        [list(Self::distinct(self.pieces.iter().map(|p| p.item))), list(Self::distinct(self.props.iter().map(|p| p.item)))]
    }

    /// The file's bytes: the pieces tinted (format 2), the props, the kept blocks and tail, the start first in the tail.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let put32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_be_bytes());
        let putf = |out: &mut Vec<u8>, v: f32| out.extend_from_slice(&v.to_bits().to_be_bytes());
        let pose = |out: &mut Vec<u8>, p: Vec3, r: Quat| {
            for v in [p.x, p.y, p.z, r.x, r.y, r.z, r.w] {
                out.extend_from_slice(&v.to_bits().to_be_bytes());
            }
        };
        let mut statics = Vec::new();
        put32(&mut statics, STATIC_TINTED);
        put32(&mut statics, self.pieces.len() as u32);
        for p in &self.pieces {
            pose(&mut statics, p.position, p.rotation);
            statics.extend_from_slice(&p.item.to_be_bytes());
            for v in p.tint {
                putf(&mut statics, v);
            }
        }
        let mut props = Vec::new();
        put32(&mut props, 1);
        put32(&mut props, self.props.len() as u32);
        for p in &self.props {
            pose(&mut props, p.position, p.rotation);
            props.extend_from_slice(&p.item.to_be_bytes());
            put32(&mut props, p.flag);
        }
        let [block3, block4] = self.id_lists();
        put32(&mut out, self.version);
        for len in [statics.len(), props.len(), block3.len(), block4.len()] {
            put32(&mut out, len as u32);
        }
        out.extend_from_slice(&self.origin.to_be_bytes());
        for b in [&statics, &props, &block3, &block4] {
            out.extend_from_slice(b);
        }
        if let Some(m) = self.start {
            for row in m {
                for v in row {
                    putf(&mut out, v);
                }
            }
        }
        out.extend_from_slice(&self.tail);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ParkFile {
        ParkFile {
            version: VERSION,
            origin: 0x82C3_CE61_708B_D17D,
            pieces: vec![
                Piece { position: Vec3::new(1.0, 2.0, 3.0), rotation: Quat::from_rotation_y(0.5), item: 0x2C70_1703_0003_01B6, tint: [1.0; 3] },
                Piece { position: Vec3::new(-4.0, 0.0, 9.5), rotation: Quat::IDENTITY, item: 0x0001_DC34_03E3_8703, tint: [0.5, 0.25, 1.0] },
            ],
            props: vec![Prop { position: Vec3::new(7.0, 0.1, -2.0), rotation: Quat::from_rotation_y(-1.0), item: 0x0001_72D4_03E3_8703, flag: 1 }],
            start: Some([[0.0, 0.0, -1.0, 0.0], [0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [-115.5, 3.25, 10.5, 1.0]]),
            tail: vec![9; 20],
        }
    }

    #[test]
    fn a_layout_reads_back_as_written() {
        let file = sample();
        assert_eq!(ParkFile::parse(&file.to_bytes()).unwrap(), file);
    }

    /// Every stock layout on the disc reads; its third and fourth blocks are the id lists written; written, it reads back
    /// the same. Data-gated: `SKATE3_ASSET_ROOT` (the stock `.park` files under `private/stock/data/skatepark`).
    #[test]
    #[ignore = "needs the private install data"]
    fn the_stock_layouts_read_and_write_back() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let dir = assets.join("private/stock/data/skatepark");
        let mut read = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("park") {
                continue;
            }
            let raw = std::fs::read(&path).unwrap();
            let file = ParkFile::parse(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            println!("{}: v{} {} pieces, {} props, start {:?}", path.display(), file.version, file.pieces.len(), file.props.len(), file.start.map(|m| m[3]));
            let len = |k: usize| u32::from_be_bytes(raw[4 + k * 4..8 + k * 4].try_into().unwrap()) as usize;
            let at3 = 28 + len(0) + len(1);
            assert_eq!(file.id_lists(), [raw[at3..at3 + len(2)].to_vec(), raw[at3 + len(2)..at3 + len(2) + len(3)].to_vec()], "{}", path.display());
            assert_eq!(ParkFile::parse(&file.to_bytes()).unwrap(), file, "{}", path.display());
            read += 1;
        }
        assert!(read >= 8);
    }
}
