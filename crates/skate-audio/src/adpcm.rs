//! skate_vita: compact sample storage for the Vita.
//!
//! The reference keeps every decoded sample as `f32` (`mixer::Pcm`); the Vita's board set alone
//! would take about 209 MB that way. Here a channel can instead hold 4-bit IMA ADPCM in blocks of
//! [`BLOCK_FRAMES`] samples. Each block stores the coder state it starts from, so any block decodes
//! on its own (random access for the resampler and the grain player) and decodes to exactly what a
//! continuous decode gives. Decoded samples become `f32` as the reference converts its PCM16
//! (`f32::from(i16) / 32768.0`), so the only difference from the reference is ADPCM's own error.
//!
//! Block layout (36 bytes): predictor `i16` LE, step index `u8`, padding `u8`, then 32 bytes of
//! nibbles, low nibble first.
use std::sync::Arc;

pub const BLOCK_FRAMES: usize = 64;
pub const BLOCK_BYTES: usize = 4 + BLOCK_FRAMES / 2;

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157,
    173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552,
    1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487,
    12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// One decoder step: the next predictor and step index for `nibble`.
#[inline(always)]
fn step(predictor: i32, index: i32, nibble: u8) -> (i32, i32) {
    let s = STEPS[index as usize];
    let mut diff = s >> 3;
    if nibble & 4 != 0 {
        diff += s;
    }
    if nibble & 2 != 0 {
        diff += s >> 1;
    }
    if nibble & 1 != 0 {
        diff += s >> 2;
    }
    let p = if nibble & 8 != 0 { predictor - diff } else { predictor + diff };
    (p.clamp(-32768, 32767), (index + INDEX[usize::from(nibble & 7)]).clamp(0, 88))
}

/// One channel of IMA ADPCM.
#[derive(Clone, Debug)]
pub struct Adpcm {
    pub frames: usize,
    /// The blocks are `bytes[start..start + len]` (an `.ima` file shares its mounted bytes).
    bytes: Arc<[u8]>,
    start: usize,
}

impl PartialEq for Adpcm {
    fn eq(&self, other: &Self) -> bool {
        self.frames == other.frames && self.blocks() == other.blocks()
    }
}

impl Adpcm {
    /// The encoded blocks (`BLOCK_BYTES` per `BLOCK_FRAMES` frames).
    pub fn blocks(&self) -> &[u8] {
        &self.bytes[self.start..self.start + self.frames.div_ceil(BLOCK_FRAMES) * BLOCK_BYTES]
    }

    /// Encodes PCM16 (the bake). The encoder runs the decoder alongside, so each block's stored
    /// state is exactly where a decoder arrives.
    pub fn encode(pcm: &[i16]) -> Self {
        let mut out = Vec::with_capacity(pcm.len().div_ceil(BLOCK_FRAMES) * BLOCK_BYTES);
        let start = i32::from(pcm.first().copied().unwrap_or(0));
        // The starting step size: the one that tracks the first block best (a fixed small step
        // lags behind a loud start for a few samples).
        let first = &pcm[..pcm.len().min(BLOCK_FRAMES)];
        let error = |index: i32| {
            let (mut p, mut i, mut e) = (start, index, 0i64);
            for &x in first {
                let code = Self::code(p, i, x);
                (p, i) = step(p, i, code);
                e += i64::from(p - i32::from(x)).pow(2);
            }
            e
        };
        let (mut predictor, mut index) = (start, (0..89).min_by_key(|&i| error(i)).unwrap_or(0));
        for chunk in pcm.chunks(BLOCK_FRAMES) {
            out.extend_from_slice(&(predictor as i16).to_le_bytes());
            out.push(index as u8);
            out.push(0);
            let mut nibbles = [0u8; BLOCK_FRAMES];
            for (n, &x) in nibbles.iter_mut().zip(chunk) {
                let code = Self::code(predictor, index, x);
                (predictor, index) = step(predictor, index, code);
                *n = code;
            }
            for pair in nibbles.chunks(2) {
                out.push(pair[0] | (pair[1] << 4));
            }
        }
        Self { frames: pcm.len(), bytes: out.into(), start: 0 }
    }

    /// The standard IMA encoder's nibble for `x` from this state.
    fn code(predictor: i32, index: i32, x: i16) -> u8 {
        let mut diff = i32::from(x) - predictor;
        let mut code = 0u8;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        let mut threshold = STEPS[index as usize];
        for bit in [4u8, 2, 1] {
            if diff >= threshold {
                code |= bit;
                diff -= threshold;
            }
            threshold >>= 1;
        }
        code
    }

    /// Decodes block `b` into `out` (as `f32`, the reference's PCM16 scale).
    pub fn decode_block(&self, b: usize, out: &mut [f32; BLOCK_FRAMES]) {
        let at = b * BLOCK_BYTES;
        let Some(block) = self.blocks().get(at..at + BLOCK_BYTES) else {
            *out = [0.0; BLOCK_FRAMES];
            return;
        };
        decode_block_bytes(block, out);
    }

    /// Every sample (tests, and callers that need a whole channel).
    pub fn decode_all(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.frames);
        let mut block = [0.0f32; BLOCK_FRAMES];
        for b in 0..self.frames.div_ceil(BLOCK_FRAMES) {
            self.decode_block(b, &mut block);
            let n = (self.frames - b * BLOCK_FRAMES).min(BLOCK_FRAMES);
            out.extend_from_slice(&block[..n]);
        }
        out
    }
}

/// Decodes one `BLOCK_BYTES` block (predictor, index, 64 nibbles) as `f32` (the reference's PCM16 scale).
pub(crate) fn decode_block_bytes(block: &[u8], out: &mut [f32; BLOCK_FRAMES]) {
    let mut predictor = i32::from(i16::from_le_bytes([block[0], block[1]]));
    let mut index = i32::from(block[2]).min(88);
    for (k, o) in out.iter_mut().enumerate() {
        let byte = block[4 + k / 2];
        let nibble = if k % 2 == 0 { byte & 15 } else { byte >> 4 };
        (predictor, index) = step(predictor, index, nibble);
        *o = f32::from(predictor as i16) / 32768.0;
    }
}

/// One channel's samples: the reference's `f32`, IMA ADPCM in memory, or IMA ADPCM read ahead from
/// a file on the card (the Vita's ambience beds).
#[derive(Clone, Debug)]
pub enum Samples {
    F32(Vec<f32>),
    Adpcm(Adpcm),
    Stream(Arc<crate::stream::StreamSource>),
}

impl PartialEq for Samples {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Samples::F32(a), Samples::F32(b)) => a == b,
            (Samples::Adpcm(a), Samples::Adpcm(b)) => a == b,
            (Samples::Stream(a), Samples::Stream(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl From<Vec<f32>> for Samples {
    fn from(v: Vec<f32>) -> Self {
        Samples::F32(v)
    }
}

impl Samples {
    pub fn len(&self) -> usize {
        match self {
            Samples::F32(v) => v.len(),
            Samples::Adpcm(a) => a.frames,
            Samples::Stream(s) => s.frames,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `f32` samples, when stored that way.
    pub fn as_f32(&self) -> Option<&[f32]> {
        match self {
            Samples::F32(v) => Some(v),
            Samples::Adpcm(_) | Samples::Stream(_) => None,
        }
    }

    /// Sample `i` (decodes its block: for occasional reads; streams use [`Reader`]).
    pub fn get(&self, i: usize) -> Option<f32> {
        match self {
            Samples::F32(v) => v.get(i).copied(),
            Samples::Adpcm(a) => (i < a.frames).then(|| {
                let mut block = [0.0f32; BLOCK_FRAMES];
                a.decode_block(i / BLOCK_FRAMES, &mut block);
                block[i % BLOCK_FRAMES]
            }),
            Samples::Stream(s) => (i < s.frames).then(|| {
                let mut block = [0.0f32; BLOCK_FRAMES];
                s.decode(i / BLOCK_FRAMES, &mut None, &mut block);
                block[i % BLOCK_FRAMES]
            }),
        }
    }

    pub fn reader(&self) -> Reader<'_> {
        Reader { samples: self, block: usize::MAX, buf: [0.0; BLOCK_FRAMES], chunk: None }
    }
}

/// Sequential-friendly access: keeps the last decoded ADPCM block.
pub struct Reader<'a> {
    samples: &'a Samples,
    block: usize,
    buf: [f32; BLOCK_FRAMES],
    /// A stream's chunk this reader is in.
    chunk: Option<(usize, Arc<[u8]>)>,
}

impl Reader<'_> {
    #[inline]
    pub fn get(&mut self, i: usize) -> Option<f32> {
        match self.samples {
            Samples::F32(v) => v.get(i).copied(),
            Samples::Adpcm(a) => {
                if i >= a.frames {
                    return None;
                }
                let b = i / BLOCK_FRAMES;
                if b != self.block {
                    a.decode_block(b, &mut self.buf);
                    self.block = b;
                }
                Some(self.buf[i % BLOCK_FRAMES])
            }
            Samples::Stream(s) => {
                if i >= s.frames {
                    return None;
                }
                let b = i / BLOCK_FRAMES;
                if b != self.block {
                    s.decode(b, &mut self.chunk, &mut self.buf);
                    self.block = b;
                }
                Some(self.buf[i % BLOCK_FRAMES])
            }
        }
    }
}

/// The `.ima` file (vita-bake audio): `SKIM`, version, rate, channels, frames, then per channel
/// its block bytes with their length. One decoded WAV of the reference's audio export.
const MAGIC: &[u8; 4] = b"SKIM";
const FILE_VERSION: u32 = 1;

/// Encodes planar PCM16 channels of one sample as an `.ima` file.
pub fn to_ima(rate: u32, channels: &[Vec<i16>]) -> Vec<u8> {
    let frames = channels.first().map_or(0, Vec::len);
    let mut out = MAGIC.to_vec();
    for v in [FILE_VERSION, rate, channels.len() as u32, frames as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for c in channels {
        let a = Adpcm::encode(c);
        out.extend_from_slice(&(a.blocks().len() as u32).to_le_bytes());
        out.extend_from_slice(a.blocks());
    }
    out
}

/// Reads an `.ima` file as a [`crate::mixer::Pcm`] whose channels stay ADPCM.
pub fn pcm_from_ima(bytes: &[u8]) -> Result<crate::mixer::Pcm, String> {
    let len = bytes.len();
    pcm_from_ima_shared(bytes.into(), 0..len)
}

/// [`pcm_from_ima`] of `bytes[range]` without a copy: the channels keep a reference to `bytes`.
pub fn pcm_from_ima_shared(bytes: Arc<[u8]>, range: std::ops::Range<usize>) -> Result<crate::mixer::Pcm, String> {
    let file = bytes.get(range.clone()).ok_or("an .ima range outside its buffer")?;
    let word = |at: usize| file.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or("truncated .ima");
    if file.get(..4) != Some(&MAGIC[..]) || word(4)? != FILE_VERSION {
        return Err("not an .ima file of this version".into());
    }
    let (rate, count, frames) = (word(8)?, word(12)? as usize, word(16)? as usize);
    let mut at = 20;
    let mut channels = Vec::with_capacity(count);
    for _ in 0..count {
        let len = word(at)? as usize;
        file.get(at + 4..at + 4 + len).ok_or("truncated .ima channel")?;
        if len != frames.div_ceil(BLOCK_FRAMES) * BLOCK_BYTES {
            return Err("an .ima channel's size does not match its frames".into());
        }
        channels.push(Samples::Adpcm(Adpcm { frames, bytes: bytes.clone(), start: range.start + at + 4 }));
        at += 4 + len;
    }
    Ok(crate::mixer::Pcm { rate, channels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_decode_alone_and_track_the_signal() {
        let pcm: Vec<i16> = (0..10_000).map(|i| ((i as f32 * 0.031).sin() * 12_000.0 + (i as f32 * 0.17).sin() * 3_000.0) as i16).collect();
        let a = Adpcm::encode(&pcm);
        assert_eq!(a.blocks().len(), pcm.len().div_ceil(BLOCK_FRAMES) * BLOCK_BYTES);
        let all = a.decode_all();
        assert_eq!(all.len(), pcm.len());
        // Error against the PCM16 source: a few percent of the signal at most.
        let err = all.iter().zip(&pcm).map(|(d, &s)| (d - f32::from(s) / 32768.0).abs()).fold(0.0f32, f32::max);
        let first = all.iter().zip(&pcm).position(|(d, &s)| (d - f32::from(s) / 32768.0).abs() > 0.03);
        let (sig, noise) = all.iter().zip(&pcm).fold((0.0f64, 0.0f64), |(a, b), (d, &s)| {
            let x = f64::from(s) / 32768.0;
            (a + x * x, b + (f64::from(*d) - x).powi(2))
        });
        let snr = 10.0 * (sig / noise).log10();
        eprintln!("max err {err}, first over 0.03 at {first:?}, SNR {snr:.1} dB");
        assert!(snr > 40.0 && err < 0.01, "SNR {snr} dB, max error {err}");
        // Random access equals the continuous decode.
        let s = Samples::Adpcm(a);
        let mut r = s.reader();
        for i in [9_999usize, 0, 64, 63, 5_000, 4_999] {
            assert_eq!(s.get(i), Some(all[i]));
            assert_eq!(r.get(i), Some(all[i]));
        }
        assert_eq!(s.get(10_000), None);
    }

    #[test]
    fn ima_files_round_trip() {
        let left: Vec<i16> = (0..1000).map(|i| (i * 37 % 2000 - 1000) as i16).collect();
        let right: Vec<i16> = left.iter().map(|x| -x).collect();
        let pcm = pcm_from_ima(&to_ima(44_100, &[left.clone(), right])).unwrap();
        assert_eq!((pcm.rate, pcm.channels.len(), pcm.channels[1].len()), (44_100, 2, 1000));
        let Samples::Adpcm(a) = &pcm.channels[0] else { panic!() };
        assert_eq!(a, &Adpcm::encode(&left));
        assert!(pcm_from_ima(b"SKIM").is_err());
    }
}
