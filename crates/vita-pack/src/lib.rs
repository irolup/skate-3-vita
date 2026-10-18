//! Baked asset packs, shared by `tools/vita-bake` (desktop) and the game.
//!
//! A pack is one container: a 56-byte header (magic, kind, kind version, payload
//! length, blake3 hash of the payload) and a little-endian payload whose layout the
//! kind's module defines. Packs hold retail-derived data and are never committed.
pub mod district;
pub mod files;
pub mod map;
pub mod skater;
pub mod texture;
mod wire;

pub use wire::{Reader, Writer};

/// Container magic and layout version.
pub const MAGIC: [u8; 8] = *b"SKVPACK1";
pub const HEADER_LEN: usize = 56;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Skater,
    Files,
    Map,
    District,
    /// The movable props' render (`map::PropRender`).
    Props,
}
impl Kind {
    fn code(self) -> u32 {
        match self {
            Kind::Skater => 1,
            Kind::Files => 2,
            Kind::Map => 3,
            Kind::District => 4,
            Kind::Props => 5,
        }
    }
    fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Kind::Skater),
            2 => Some(Kind::Files),
            3 => Some(Kind::Map),
            4 => Some(Kind::District),
            5 => Some(Kind::Props),
            _ => None,
        }
    }
}

/// Wraps a payload in the container.
pub fn seal(kind: Kind, version: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&kind.code().to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(blake3::hash(payload).as_bytes());
    out.extend_from_slice(payload);
    out
}

/// Checks the container and returns (kind, version, payload).
pub fn open(bytes: &[u8]) -> Result<(Kind, u32, &[u8]), String> {
    let opened = open_unverified(bytes)?;
    verify(bytes)?;
    Ok(opened)
}

/// The header and length checks of [`open`], without hashing the payload (the caller
/// runs [`verify`] separately, e.g. on another core while loading).
pub fn open_unverified(bytes: &[u8]) -> Result<(Kind, u32, &[u8]), String> {
    if bytes.len() < HEADER_LEN || bytes[..8] != MAGIC {
        return Err("not a skate vita pack (bad magic)".into());
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let kind = Kind::from_code(word(8)).ok_or_else(|| format!("unknown pack kind {}", word(8)))?;
    let version = word(12);
    let length = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    let payload = bytes
        .get(HEADER_LEN..HEADER_LEN + length)
        .filter(|_| bytes.len() == HEADER_LEN + length)
        .ok_or("pack length does not match its header")?;
    Ok((kind, version, payload))
}

/// [`verify`] of a pack file read in 1 MiB pieces (the Vita keeps its packs on the
/// card instead of in memory).
pub fn verify_file(path: &std::path::Path) -> Result<(), String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        // Out of the way of a loader reading from the card (skate_data::io_gate).
        skate_data::io_gate::wait_background(std::time::Duration::from_secs(1));
        let n = file.read(&mut chunk).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    if hasher.finalize().as_bytes() != &header[24..56] {
        return Err("pack payload hash mismatch (damaged or truncated file)".into());
    }
    Ok(())
}

/// The payload's blake3 hash against the header (after `open_unverified` succeeded).
pub fn verify(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < HEADER_LEN || blake3::hash(&bytes[HEADER_LEN..]).as_bytes() != &bytes[24..56] {
        return Err("pack payload hash mismatch (damaged or truncated file)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_round_trips_and_rejects_damage() {
        let packed = seal(Kind::Skater, 7, b"payload bytes");
        assert_eq!(open(&packed).unwrap(), (Kind::Skater, 7, &b"payload bytes"[..]));
        let mut damaged = packed.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(open(&damaged).unwrap_err().contains("hash"));
        assert!(open(&packed[..packed.len() - 1]).is_err());
        assert!(open(b"not a pack at all, definitely not one, no").is_err());
    }
}
