//! Little-endian payload encoding.

#[derive(Default)]
pub struct Writer {
    pub bytes: Vec<u8>,
}
impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn str(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.bytes.extend_from_slice(s.as_bytes());
    }
    /// A length-prefixed byte block (u32 length).
    pub fn block(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.bytes.extend_from_slice(b);
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or_else(|| {
            format!("pack payload truncated at byte {} (+{n})", self.at)
        })?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn str(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|e| e.to_string())
    }
    pub fn block(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub fn done(&self) -> Result<(), String> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(format!("{} unread bytes at the end of the pack", self.bytes.len() - self.at))
        }
    }
}

/// What the sim part of a pack decodes from: the bytes in memory ([`Reader`]) or, for a district's index, the card
/// ([`StreamReader`]).
pub trait Source {
    fn u8(&mut self) -> Result<u8, String>;
    fn u16(&mut self) -> Result<u16, String>;
    fn u32(&mut self) -> Result<u32, String>;
    fn u64(&mut self) -> Result<u64, String>;
    fn i32(&mut self) -> Result<i32, String>;
    fn f32(&mut self) -> Result<f32, String>;
    fn str(&mut self) -> Result<String, String>;
    fn block_vec(&mut self) -> Result<Vec<u8>, String>;
    /// A map extension's payload (a length-prefixed block), as the game will keep it.
    fn extension_payload(&mut self, tag: [u8; 4], schema: u32) -> Result<Vec<u8>, String>;
}

impl Source for Reader<'_> {
    fn u8(&mut self) -> Result<u8, String> {
        Reader::u8(self)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Reader::u16(self)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Reader::u32(self)
    }
    fn u64(&mut self) -> Result<u64, String> {
        Reader::u64(self)
    }
    fn i32(&mut self) -> Result<i32, String> {
        Reader::i32(self)
    }
    fn f32(&mut self) -> Result<f32, String> {
        Reader::f32(self)
    }
    fn str(&mut self) -> Result<String, String> {
        Reader::str(self)
    }
    fn block_vec(&mut self) -> Result<Vec<u8>, String> {
        Ok(self.block()?.to_vec())
    }
    fn extension_payload(&mut self, _tag: [u8; 4], _schema: u32) -> Result<Vec<u8>, String> {
        self.block_vec()
    }
}

/// [`Source`] over `len` bytes of a stream, so that decoding needs no block as long as the whole payload. A district's
/// index in one block, and its `WMET` copied out of that, took 25 MB of unbroken memory while changing district; with
/// 100 MB of the heap free in pieces, the 9.7 MB copy did not fit and the game aborted.
pub struct StreamReader<R> {
    inner: R,
    len: u64,
    left: u64,
}

impl<R: std::io::Read> StreamReader<R> {
    pub fn new(inner: R, len: u64) -> Self {
        Self { inner, len, left: len }
    }

    /// Bytes read so far.
    pub fn at(&self) -> u64 {
        self.len - self.left
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    pub fn done(&self) -> Result<(), String> {
        if self.left == 0 {
            Ok(())
        } else {
            Err(format!("{} unread bytes at the end of the pack", self.left))
        }
    }

    /// Takes `n` bytes off what is left, before anything is allocated for them (a damaged length).
    fn reserve(&mut self, n: u64) -> Result<(), String> {
        if n > self.left {
            return Err(format!("pack payload truncated at byte {} (+{n})", self.at()));
        }
        self.left -= n;
        Ok(())
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.reserve(N as u64)?;
        let mut bytes = [0; N];
        self.inner.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, String> {
        self.reserve(n as u64)?;
        let mut bytes = vec![0; n];
        self.inner.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
}

impl<R: std::io::Read> Source for StreamReader<R> {
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.fixed::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.fixed()?))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.fixed()?))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.fixed()?))
    }
    fn str(&mut self) -> Result<String, String> {
        let n = Source::u16(self)? as usize;
        String::from_utf8(self.bytes(n)?).map_err(|e| e.to_string())
    }
    fn block_vec(&mut self) -> Result<Vec<u8>, String> {
        let n = Source::u32(self)? as usize;
        self.bytes(n)
    }
    /// `WMET` is filtered as it goes by (`map::slim_wmet`); it is most of a district's index.
    fn extension_payload(&mut self, tag: [u8; 4], schema: u32) -> Result<Vec<u8>, String> {
        if &tag != b"WMET" || schema != 1 {
            return self.block_vec();
        }
        let n = u64::from(Source::u32(self)?);
        self.reserve(n)?;
        let mut json = std::io::Read::take(&mut self.inner, n);
        let slim = crate::map::slim_wmet(&mut json, n as usize / 4)?;
        std::io::copy(&mut json, &mut std::io::sink()).map_err(|e| e.to_string())?;
        if json.limit() != 0 {
            return Err("WMET: the file ends inside it".into());
        }
        Ok(slim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_reads_what_the_slice_reads_and_refuses_too_long_lengths() {
        let mut w = Writer::default();
        w.u8(1);
        w.u16(2);
        w.u32(3);
        w.u64(4);
        w.i32(-5);
        w.f32(6.5);
        w.str("seven");
        w.block(&[8, 8]);
        let mut r = StreamReader::new(&w.bytes[..], w.bytes.len() as u64);
        assert_eq!(Source::u8(&mut r).unwrap(), 1);
        assert_eq!(Source::u16(&mut r).unwrap(), 2);
        assert_eq!(Source::u32(&mut r).unwrap(), 3);
        assert_eq!(Source::u64(&mut r).unwrap(), 4);
        assert_eq!(Source::i32(&mut r).unwrap(), -5);
        assert_eq!(Source::f32(&mut r).unwrap(), 6.5);
        assert!(r.done().is_err());
        assert_eq!(Source::str(&mut r).unwrap(), "seven");
        assert_eq!(r.block_vec().unwrap(), vec![8, 8]);
        r.done().unwrap();
        assert!(Source::u8(&mut r).is_err());
        // A length past the end fails before it allocates.
        let mut w = Writer::default();
        w.u32(u32::MAX);
        assert!(StreamReader::new(&w.bytes[..], 4).block_vec().is_err());
    }
}
