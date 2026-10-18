//! Texture baking: power-of-two sides at most 256, a full mip chain,
//! DXT1 (opaque) or DXT5 (alpha) with texpresso.
use image::{RgbaImage, imageops::FilterType};
use vita_pack::texture::{Format, Texture};

pub const MAX_SIDE: u32 = 256;

fn side(v: u32) -> u32 {
    let v = v.clamp(1, MAX_SIDE);
    if v.is_power_of_two() { v } else { v.next_power_of_two() / 2 }
}

pub fn bake(rgba: &[u8], width: u32, height: u32, format: Format) -> Result<Texture, String> {
    let source = RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or("texture size does not match its pixels")?;
    let (w, h) = (side(width), side(height));
    let mut level = if (w, h) == (width, height) {
        source
    } else {
        image::imageops::resize(&source, w, h, FilterType::Lanczos3)
    };
    let mut mips = vec![encode(&level, format)];
    while level.width() > 1 || level.height() > 1 {
        let (nw, nh) = ((level.width() / 2).max(1), (level.height() / 2).max(1));
        level = image::imageops::resize(&level, nw, nh, FilterType::Triangle);
        mips.push(encode(&level, format));
    }
    let t = Texture { width: w as u16, height: h as u16, format, mips };
    t.validate()?;
    Ok(t)
}

fn encode(level: &RgbaImage, format: Format) -> Vec<u8> {
    let (w, h) = (level.width() as usize, level.height() as usize);
    let codec = match format {
        Format::Rgba8 => return level.as_raw().clone(),
        Format::Dxt1 => texpresso::Format::Bc1,
        Format::Dxt5 => texpresso::Format::Bc3,
    };
    let mut out = vec![0u8; codec.compressed_size(w, h)];
    codec.compress(level.as_raw(), w, h, texpresso::Params::default(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_are_power_of_two_capped_with_full_dxt_mips() {
        let rgba = vec![200u8; 300 * 100 * 4];
        let t = bake(&rgba, 300, 100, Format::Dxt1).unwrap();
        assert_eq!((t.width, t.height), (256, 64));
        assert_eq!(t.mips.len(), 9); // 256 .. 1
        assert_eq!(t.mips[0].len(), 64 * 16 * 8);
        let t = bake(&vec![9u8; 4 * 4 * 4], 4, 4, Format::Dxt5).unwrap();
        assert_eq!(t.mips.iter().map(Vec::len).collect::<Vec<_>>(), [16, 16, 16]);
    }
}
