//! Textures as the Vita uploads them (at most 256 × 256, compressed,
//! mipmapped): RGBA8, or S3TC DXT1 (opaque) / DXT5 (with alpha), level 0 first.
use crate::{Reader, Writer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgba8,
    Dxt1,
    Dxt5,
}

impl Format {
    fn code(self) -> u8 {
        match self {
            Format::Rgba8 => 0,
            Format::Dxt1 => 1,
            Format::Dxt5 => 2,
        }
    }
    fn from_code(c: u8) -> Result<Self, String> {
        Ok(match c {
            0 => Format::Rgba8,
            1 => Format::Dxt1,
            2 => Format::Dxt5,
            _ => return Err(format!("unknown texture format {c}")),
        })
    }
    /// Bytes of one level of `width` × `height` pixels.
    pub fn level_size(self, width: usize, height: usize) -> usize {
        let blocks = width.div_ceil(4) * height.div_ceil(4);
        match self {
            Format::Rgba8 => width * height * 4,
            Format::Dxt1 => blocks * 8,
            Format::Dxt5 => blocks * 16,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub width: u16,
    pub height: u16,
    pub format: Format,
    /// Level 0 first, each half the previous (minimum 1).
    pub mips: Vec<Vec<u8>>,
}

impl Texture {
    pub fn bytes(&self) -> usize {
        self.mips.iter().map(Vec::len).sum()
    }

    pub fn write(&self, w: &mut Writer) {
        w.u16(self.width);
        w.u16(self.height);
        w.u8(self.format.code());
        w.u8(self.mips.len() as u8);
        for level in &self.mips {
            w.block(level);
        }
    }

    pub fn read(r: &mut Reader) -> Result<Self, String> {
        let width = r.u16()?;
        let height = r.u16()?;
        let format = Format::from_code(r.u8()?)?;
        let mips = (0..r.u8()?)
            .map(|_| r.block().map(<[u8]>::to_vec))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { width, height, format, mips })
    }

    pub fn validate(&self) -> Result<(), String> {
        let (mut w, mut h) = (usize::from(self.width), usize::from(self.height));
        if self.mips.is_empty() {
            return Err("texture without levels".into());
        }
        for level in &self.mips {
            if level.len() != self.format.level_size(w, h) {
                return Err("texture level size does not match its dimensions".into());
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_sizes() {
        assert_eq!(Format::Rgba8.level_size(2, 2), 16);
        assert_eq!(Format::Dxt1.level_size(256, 256), 32768);
        assert_eq!(Format::Dxt5.level_size(256, 256), 65536);
        assert_eq!(Format::Dxt1.level_size(1, 1), 8);
        assert_eq!(Format::Dxt5.level_size(2, 8), 32);
    }
}
