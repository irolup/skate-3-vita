//! skate_vita addition (`sim.pak`): binary forms of the converted JSON
//! files, written by `tools/vita-bake` on the desktop from the values `serde_json`
//! parsed there, and accepted by the same loaders in place of the JSON (the Vita then
//! does no JSON parsing). Every value is stored as-is (strings as UTF-8, numbers in
//! their parsed integer form), so decoding gives exactly what parsing the JSON gave;
//! the loaders' validation runs afterwards either way.
//!
//! Layout: `MAGIC`, a 4-byte kind tag, then the kind's fields in little-endian.

pub const MAGIC: &[u8; 8] = b"SKVBIN01";

/// Whether `bytes` is a binary form (and of which kind) rather than JSON.
pub fn kind(bytes: &[u8]) -> Option<[u8; 4]> {
    (bytes.len() >= 12 && &bytes[..8] == MAGIC).then(|| bytes[8..12].try_into().unwrap())
}

pub struct Writer {
    pub bytes: Vec<u8>,
}
impl Writer {
    pub fn new(kind: &[u8; 4]) -> Self {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(kind);
        Self { bytes }
    }
    pub fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.bytes.extend_from_slice(s.as_bytes());
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    /// Checks the magic and kind, positioned after them.
    pub fn new(bytes: &'a [u8], expected: &[u8; 4]) -> Result<Self, String> {
        match kind(bytes) {
            Some(k) if &k == expected => Ok(Self { bytes, at: 12 }),
            _ => Err(format!(
                "not a binary {} record",
                String::from_utf8_lossy(expected)
            )),
        }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or("binary record truncated")?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn str(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|e| e.to_string())
    }
    pub fn done(&self) -> Result<(), String> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err("trailing bytes after binary record".into())
        }
    }
}
