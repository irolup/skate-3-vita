//! `sim.pak` and `anim.pak` (kind `Files`): named files for the asset loaders' virtual
//! file system (`skate_data::vfs`), keyed by their path relative to the asset root.
//! `sim.pak` holds the simulation data (the converted JSON files in their binary form,
//! the retail graphs, PAT and camera files as-is); `anim.pak` the two ABIN banks as-is.
use crate::{Kind, Reader, Writer};
use std::sync::Arc;

pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Relative to the asset root, `/`-separated (e.g. `private/game.json`).
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn to_pack(files: &[File]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u32(files.len() as u32);
    for f in files {
        w.str(&f.path);
        w.block(&f.bytes);
    }
    crate::seal(Kind::Files, VERSION, &w.bytes)
}

/// The files of a pack, each as shared bytes ready to mount.
pub fn from_pack(bytes: &[u8]) -> Result<Vec<(String, Arc<[u8]>)>, String> {
    crate::verify(bytes).and_then(|()| from_pack_unverified(bytes))
}

/// [`from_pack`] without the payload hash (see `crate::open_unverified`).
pub fn from_pack_unverified(bytes: &[u8]) -> Result<Vec<(String, Arc<[u8]>)>, String> {
    let (kind, version, payload) = crate::open_unverified(bytes)?;
    if kind != Kind::Files || version != VERSION {
        return Err(format!("expected a file pack v{VERSION}, found {kind:?} v{version}"));
    }
    let mut r = Reader::new(payload);
    let files = (0..r.u32()?)
        .map(|_| -> Result<(String, Arc<[u8]>), String> { Ok((r.str()?, Arc::from(r.block()?))) })
        .collect::<Result<Vec<_>, _>>()?;
    r.done()?;
    Ok(files)
}

/// A file pack's index: (name, offset in the pack file, length) per file.
pub type Index = Vec<(String, u64, usize)>;

/// The index of a whole file pack in memory (offsets are absolute in the pack file).
pub fn index_of(pack: &[u8]) -> Result<Index, String> {
    let (kind, version, payload) = crate::open_unverified(pack)?;
    if kind != Kind::Files || version != VERSION {
        return Err(format!("expected a file pack v{VERSION}, found {kind:?} v{version}"));
    }
    let mut r = Reader::new(payload);
    let mut at = crate::HEADER_LEN as u64 + 4;
    let mut entries = Vec::new();
    for _ in 0..r.u32()? {
        let name = r.str()?;
        let block = r.block()?;
        at += 2 + name.len() as u64 + 4;
        entries.push((name, at, block.len()));
        at += block.len() as u64;
    }
    r.done()?;
    Ok(entries)
}

/// The index file next to a pack (`<pack>.idx`): the pack's payload hash (the header's), then the
/// entries; the Vita reads it instead of one pass over the pack.
pub fn encode_index(hash: &[u8], entries: &[(String, u64, usize)]) -> Vec<u8> {
    let mut out = hash.to_vec();
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (name, offset, size) in entries {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(*size as u32).to_le_bytes());
    }
    out
}

/// The entries of an index file, None when it is damaged or for another pack (`hash`).
pub fn decode_index(bytes: &[u8], hash: &[u8]) -> Option<Index> {
    if bytes.get(..32)? != hash {
        return None;
    }
    let mut at = 32;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = bytes.get(at..at + n)?;
        at += n;
        Some(s)
    };
    let count = u32::from_le_bytes(take(4)?.try_into().ok()?);
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len = usize::from(u16::from_le_bytes(take(2)?.try_into().ok()?));
        let name = String::from_utf8(take(len)?.to_vec()).ok()?;
        let offset = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let size = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        entries.push((name, offset, size));
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index of a pack names each file where its bytes start, and survives the index file.
    #[test]
    fn the_index_locates_every_file() {
        let files = vec![
            File { path: "a/b.ima".into(), bytes: b"hello".to_vec() },
            File { path: "c.abk".into(), bytes: Vec::new() },
            File { path: "d".into(), bytes: b"world!".to_vec() },
        ];
        let pack = to_pack(&files);
        let index = index_of(&pack).unwrap();
        for (file, (name, offset, len)) in files.iter().zip(&index) {
            assert_eq!((&file.path, file.bytes.len()), (name, *len));
            assert_eq!(&pack[*offset as usize..*offset as usize + len], &file.bytes[..]);
        }
        let hash = &pack[24..56];
        assert_eq!(decode_index(&encode_index(hash, &index), hash), Some(index));
        assert_eq!(decode_index(&encode_index(hash, &[]), &[0u8; 32]), None);
    }

    #[test]
    fn files_round_trip() {
        let files = vec![
            File { path: "private/game.json".into(), bytes: b"abc".to_vec() },
            File { path: "private/skater.glb".into(), bytes: Vec::new() },
        ];
        let back = from_pack(&to_pack(&files)).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].0.as_str(), &*back[0].1), ("private/game.json", &b"abc"[..]));
        assert!(back[1].1.is_empty());
    }
}
