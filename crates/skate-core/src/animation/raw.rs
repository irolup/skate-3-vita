//! Stock RAW ("DR", codec word 0x00004452) SQT decoding: the uncompressed packed
//! frames the scene clips use (`scene/trickguide/*.abin`, the Trick Guide demo).
//! TU3 sampler sub_82D1E658 (VMX job after the VBR job 82D1DE08). The frame layout matches the pinned
//! skate3_anim importer (tools/vendor/skate3_anim); its packed rotation does not (see `unpack_rotation`).
//! Compression header: six channel-mask words (scale, rotation, translation;
//! two words each), u16 frame size, u8 bone count - 1, u8 packed translation,
//! u8 packed rotation. Every frame is `frame size` bytes from the data start.

pub struct RawDecoder<'a> {
    bytes: &'a [u8],
    data: usize,
    masks: [u32; 6],
    frame_size: usize,
    channels: usize,
    packed_translation: bool,
    packed_rotation: bool,
}
impl<'a> RawDecoder<'a> {
    ///`channels` comes from the part header (bits 6..11 plus 1), not the
    ///compression header's bone count.
    pub fn new(
        bytes: &'a [u8],
        part_offset: usize,
        compression_header_relative: usize,
        compressed_data_relative: usize,
        channels: usize,
    ) -> Result<Self, String> {
        let header = part_offset + compression_header_relative;
        let field = |at: usize, len: usize| {
            bytes.get(header + at..header + at + len).ok_or("RAW header outside bank")
        };
        let mut masks = [0; 6];
        for (i, mask) in masks.iter_mut().enumerate() {
            *mask = u32::from_be_bytes(field(4 * i, 4)?.try_into().unwrap());
        }
        let frame_size = u16::from_be_bytes(field(24, 2)?.try_into().unwrap()) as usize;
        let flags = field(27, 2)?;
        if channels == 0 || channels > 64 {
            return Err(format!("RAW part has {channels} channels"));
        }
        Ok(Self {
            bytes,
            data: part_offset + compressed_data_relative,
            masks,
            frame_size,
            channels,
            packed_translation: flags[0] != 0,
            packed_rotation: flags[1] != 0,
        })
    }
    pub fn channel_count(&self) -> usize {
        self.channels
    }
    ///Frames the part's data range holds; the clip header carries the authored count.
    pub fn frames_in(&self, compressed_size: usize) -> usize {
        compressed_size.checked_div(self.frame_size).unwrap_or(0)
    }
    fn has(&self, channel: usize, kind: usize) -> bool {
        self.masks[kind + (channel >> 5)] & (1 << (channel & 31)) != 0
    }

    ///Each channel's scaleXYZ, quaternionXYZW, translationXYZ bits. Channels
    ///without a stored component keep identity (scale 1, rotation (0,0,0,1), no offset).
    pub fn decode_frame(&self, frame: usize) -> Result<Vec<[u32; 10]>, String> {
        let identity = [1.0f32, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0].map(f32::to_bits);
        let mut output = vec![identity; self.channels];
        self.decode_into(frame, &mut output)?;
        Ok(output)
    }
    ///Write only the components the frame stores over `output` (one entry per channel); the rest keep
    ///what the caller put there (scene clips store rotations and take bone offsets from the file's pose).
    pub fn decode_into(&self, frame: usize, output: &mut [[u32; 10]]) -> Result<(), String> {
        if output.len() != self.channels {
            return Err("RAW output does not match the part's channels".into());
        }
        let mut at = self.data + frame * self.frame_size;
        let end = at + self.frame_size;
        let mut take = |len: usize| -> Result<&[u8], String> {
            let slice = self.bytes.get(at..at + len).filter(|_| at + len <= end)
                .ok_or_else(|| format!("RAW frame {frame} reads past its record"))?;
            at += len;
            Ok(slice)
        };
        let f32_at = |b: &[u8], i: usize| f32::from_be_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
        for (channel, sample) in output.iter_mut().enumerate() {
            let mut write = |offset: usize, values: &[f32]| {
                for (i, v) in values.iter().enumerate() {
                    sample[offset + i] = v.to_bits();
                }
            };
            if self.has(channel, 0) {
                let b = take(12)?;
                write(0, &[f32_at(b, 0), f32_at(b, 1), f32_at(b, 2)]);
            }
            if self.has(channel, 2) {
                let rotation = if self.packed_rotation {
                    unpack_rotation(u32::from_be_bytes(take(4)?.try_into().unwrap()))
                } else {
                    let b = take(16)?;
                    [f32_at(b, 0), f32_at(b, 1), f32_at(b, 2), f32_at(b, 3)]
                };
                write(3, &rotation);
            }
            if self.has(channel, 4) {
                let translation = if self.packed_translation {
                    let b = take(8)?;
                    let i16_at = |i: usize| i16::from_be_bytes([b[2 * i], b[2 * i + 1]]) as f32;
                    let range = u16::from_be_bytes([b[6], b[7]]) as f32 / 32768.0;
                    [i16_at(0) * range, i16_at(1) * range, i16_at(2) * range]
                } else {
                    let b = take(12)?;
                    [f32_at(b, 0), f32_at(b, 1), f32_at(b, 2)]
                };
                write(7, &translation);
            }
        }
        Ok(())
    }
}

///32-bit packed rotation, TU3 RAW sampler sub_82D1E658: an axis in spherical angles times the vector length,
///w = sqrt(1 - r^2) (never negative). Azimuth A = bits 21..31 * 2pi/2047 - pi, latitude M = bits 11..20 *
///pi/1023 - pi/2, r = bits 0..10 / 2047. Main-checked: the header reads, the pi constant 0x82060C44 and the
///reciprocal square root; the cos/sin order of A and the signs come from the data (a symmetric T-pose,
///`.local/research/menus-m6/raw-codec.md`). No normalisation here (the engine normalises after nlerp).
fn unpack_rotation(word: u32) -> [f32; 4] {
    use std::f32::consts::{FRAC_PI_2, PI};
    let a = ((word >> 21) & 0x7ff) as f32 * (2.0 * PI / 2047.0) - PI;
    let m = ((word >> 11) & 0x3ff) as f32 * (PI / 1023.0) - FRAC_PI_2;
    let r = (word & 0x7ff) as f32 / 2047.0;
    [r * m.cos() * a.cos(), r * m.cos() * a.sin(), r * m.sin(), (1.0 - r * r).max(0.0).sqrt()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(masks: [u32; 6], frame_size: u16, packed: (u8, u8), frames: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for m in masks { bytes.extend(m.to_be_bytes()); }
        bytes.extend(frame_size.to_be_bytes());
        bytes.extend([0, packed.0, packed.1, 0, 0, 0]);
        bytes.extend(frames);
        bytes
    }

    #[test]
    fn missing_components_stay_identity_and_floats_read_in_order() {
        // Channel 0: rotation + translation as floats; channel 1: nothing stored.
        let mut frame = Vec::new();
        for v in [0.0f32, 0.0, 0.70710677, 0.70710677, 1.0, 2.0, 3.0] { frame.extend(v.to_be_bytes()); }
        let bytes = part([0, 0, 1, 0, 1, 0], 28, (0, 0), &frame);
        let decoder = RawDecoder::new(&bytes, 0, 0, 32, 2).unwrap();
        let out = decoder.decode_frame(0).unwrap();
        let f = |w: [u32; 10]| w.map(f32::from_bits);
        assert_eq!(f(out[0]), [1.0, 1.0, 1.0, 0.0, 0.0, 0.70710677, 0.70710677, 1.0, 2.0, 3.0]);
        assert_eq!(f(out[1]), [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        assert!(decoder.decode_frame(1).is_err());
    }

    #[test]
    fn packed_translation_scales_by_range_and_packed_rotation_is_axis_angle() {
        let mut frame = Vec::new();
        frame.extend(0x4000_0000u32.to_be_bytes());
        for v in [2i16, -2, 0] { frame.extend(v.to_be_bytes()); }
        frame.extend(16384u16.to_be_bytes());
        let bytes = part([0, 0, 1, 0, 1, 0], 12, (1, 1), &frame);
        let out = RawDecoder::new(&bytes, 0, 0, 32, 1).unwrap().decode_frame(0).unwrap();
        let f = out[0].map(f32::from_bits);
        assert_eq!(&f[7..], &[1.0, -1.0, 0.0]);
        // 0x4000_0000: A = 512 steps, M = -pi/2, r = 0 -> identity.
        let n: f32 = f[3..7].iter().map(|v| v * v).sum();
        assert!((n - 1.0).abs() < 1e-5 && (f[6] - 1.0).abs() < 1e-6);
        // Full magnitude on the pole: (0, 0, 1, 0) up to rounding.
        let q = unpack_rotation((1023 << 11) | 2047);
        assert!(q[2] > 0.9999 && q[3].abs() < 1e-3);
    }
}
