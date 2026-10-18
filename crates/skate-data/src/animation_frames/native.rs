//! Bank-backed lazy animation samples. Each requested clip's headers are read
//! once and each of its keys is decoded on first use; registration indexes
//! every record without allocating every decoded frame. `trim` bounds the decoded
//! keys: the least recently used clips release theirs, which decode again,
//! word for word the same, when next used. Keys are handed out reference-counted,
//! so a release never frees a key still in use.
use super::{ClipFrames, Frames, ReferencePose, Repr, SampleWords, validate_samples};
use crate::{
    abin::{Bank, Codec, PartEntry, Reader, RecordData},
    animation_banks::AnimationBanks,
};
use skate_core::animation::vbr::{VbrDecoder, VbrStream};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU32, Ordering::Relaxed},
    },
};

struct Entry<T> {
    bank: usize,
    offset: usize,
    decoded: OnceLock<Result<T, String>>,
    /// The `NativeFrames::epoch` of the last request.
    used: AtomicU32,
}
impl<T> Entry<T> {
    fn new(bank: usize, offset: usize) -> Self {
        Self {
            bank,
            offset,
            decoded: OnceLock::new(),
            used: AtomicU32::new(0),
        }
    }
}
pub(super) struct NativeFrames {
    banks: Vec<Arc<Bank>>,
    clips: BTreeMap<String, Entry<ClipFrames>>,
    poses: BTreeMap<(usize, u64), Entry<ReferencePose>>,
    pose_names: BTreeMap<(usize, String), u64>,
    /// Advanced by every `trim`.
    epoch: AtomicU32,
}
/// Clips requested within this many `trim` epochs keep their keys.
const KEEP_EPOCHS: u32 = 2;
impl NativeFrames {
    pub(super) fn new(banks: &AnimationBanks) -> Result<Self, String> {
        let mut clips: BTreeMap<String, Entry<ClipFrames>> = BTreeMap::new();
        let mut poses = BTreeMap::new();
        let mut pose_names = BTreeMap::new();
        for (bank_index, bank) in banks.banks.iter().enumerate() {
            for record in bank.records() {
                let name = &record.header.name;
                let at = record.header.offset;
                match &record.data {
                    RecordData::Clip(_) => {
                        if clips
                            .get(name)
                            .is_some_and(|entry| entry.bank != bank_index)
                        {
                            return Err(format!(
                                "Ambiguous clip {name} in multiple animation banks"
                            ));
                        }
                        clips.insert(name.clone(), Entry::new(bank_index, at));
                    }
                    RecordData::Pose(_) => {
                        poses.insert((bank_index, at as u64), Entry::new(bank_index, at));
                        pose_names.insert((bank_index, name.clone()), at as u64);
                    }
                    _ => {}
                }
            }
        }
        Ok(Self {
            banks: banks.banks.clone(),
            clips,
            poses,
            pose_names,
            epoch: AtomicU32::new(0),
        })
    }
    /// Releases the decoded keys of the least recently used clips (not requested in
    /// the last `KEEP_EPOCHS` epochs) while more than `budget` bytes are decoded,
    /// down to three quarters of it, then starts a new epoch. Returns the decoded
    /// bytes left.
    pub(super) fn trim(&self, budget: usize) -> usize {
        let epoch = self.epoch.load(Relaxed);
        let mut total = 0;
        let mut idle = Vec::new();
        for (name, entry) in &self.clips {
            if let Some(Ok(clip)) = entry.decoded.get() {
                let bytes = clip.frames.decoded_bytes();
                total += bytes;
                let used = entry.used.load(Relaxed);
                if bytes > 0 && epoch.wrapping_sub(used) >= KEEP_EPOCHS {
                    idle.push((epoch.wrapping_sub(used), name.clone()));
                }
            }
        }
        if total > budget {
            idle.sort_unstable_by(|a, b| b.cmp(a));
            for (_, name) in idle {
                if total <= budget / 4 * 3 {
                    break;
                }
                if let Some(Ok(clip)) = self.clips.get(&name).and_then(|e| e.decoded.get()) {
                    total -= clip.frames.release();
                }
            }
        }
        self.epoch.store(epoch.wrapping_add(1), Relaxed);
        total
    }
    pub(super) fn clip_count(&self) -> usize {
        self.clips.len()
    }
    pub(super) fn clip_names(&self) -> impl Iterator<Item = &str> {
        self.clips.keys().map(String::as_str)
    }
    pub(super) fn clip_bank(&self, name: &str) -> Result<usize, String> {
        self.clips
            .get(&name.to_ascii_uppercase())
            .map(|e| e.bank)
            .ok_or_else(|| format!("Missing stock animation clip {name}"))
    }
    pub(super) fn clip(&self, name: &str) -> Result<&ClipFrames, String> {
        let entry = self
            .clips
            .get(&name.to_ascii_uppercase())
            .ok_or_else(|| format!("Missing stock animation clip {name}"))?;
        entry.used.store(self.epoch.load(Relaxed), Relaxed);
        entry
            .decoded
            .get_or_init(|| decode_clip(&self.banks[entry.bank], entry.offset))
            .as_ref()
            .map_err(Clone::clone)
    }
    pub(super) fn named_pose(&self, bank: usize, name: &str) -> Result<&ReferencePose, String> {
        let at = self
            .pose_names
            .get(&(bank, name.to_ascii_uppercase()))
            .ok_or_else(|| format!("Missing reference pose {name} in bank {bank}"))?;
        self.reference_pose(bank, *at)
    }
    pub(super) fn reference_pose(
        &self,
        bank: usize,
        offset: u64,
    ) -> Result<&ReferencePose, String> {
        let entry = self
            .poses
            .get(&(bank, offset))
            .ok_or_else(|| format!("Missing reference pose {bank}:{offset:#x}"))?;
        entry
            .decoded
            .get_or_init(|| decode_pose(&self.banks[bank], entry.offset))
            .as_ref()
            .map_err(Clone::clone)
    }
}

fn decode_clip(bank: &Arc<Bank>, offset: usize) -> Result<ClipFrames, String> {
    let record = bank.record_at(offset).ok_or("Missing clip source record")?;
    let RecordData::Clip(clip) = &record.data else {
        return Err("Source record is not a clip".into());
    };
    require_vbr(record.header.codec())?;
    let count = f32::from_bits(clip.frame_count_bits);
    if !count.is_finite() || count < 1.0 || count.fract() != 0.0 {
        return Err(format!("{}: invalid frame count", record.header.name));
    }
    let frames = stock_keys(bank, &clip.parts, count as usize, &record.header.name)
        .map_err(|e| format!("{}: {e}", record.header.name))?;
    let bones = bank.hierarchy().ok_or("No bank hierarchy")?.bone_count as usize;
    let mut weights = vec![1.0f32.to_bits(); bones];
    if clip.channel_animation() {
        let reader = Reader::new(bank.bytes());
        for (entry, layout) in clip.parts.iter().zip(&bank.hierarchy().unwrap().parts) {
            let part = entry
                .part
                .as_ref()
                .ok_or("Channel animation part is absent")?;
            let range = part
                .channel_weights
                .as_ref()
                .ok_or("Channel animation weights are absent")?;
            for lane in 0..layout.bone_count as usize {
                let word = reader
                    .u32(range.start + lane * 4)
                    .map_err(|e| e.to_string())?;
                if !f32::from_bits(word).is_finite() {
                    return Err("Nonfinite channel weight".into());
                }
                weights[layout.sqt_offset as usize + lane] = word;
            }
        }
    }
    Ok(ClipFrames {
        name: record.header.name.clone(),
        source_offset: offset as u64,
        fps_bits: clip.fps_bits,
        loop_translation_bits: clip.loop_translation_words[..3].try_into().unwrap(),
        loop_rotation_bits: clip.loop_rotation_words,
        channel_animation: clip.channel_animation(),
        channel_weights: weights,
        frames: Frames(Repr::Stock(frames)),
    })
}
fn decode_pose(bank: &Bank, offset: usize) -> Result<ReferencePose, String> {
    let record = bank.record_at(offset).ok_or("Missing pose source record")?;
    let RecordData::Pose(pose) = &record.data else {
        return Err("Source record is not a pose".into());
    };
    require_vbr(record.header.codec())?;
    let mut frames =
        decode_parts(bank, &pose.parts, 1).map_err(|e| format!("{}: {e}", record.header.name))?;
    Ok(ReferencePose {
        name: record.header.name.clone(),
        source_offset: offset as u64,
        samples: frames.remove(0),
    })
}
fn require_vbr(codec: Codec) -> Result<(), String> {
    if codec != Codec::Vbr {
        return Err(format!("Unsupported native animation codec {codec:?}"));
    }
    Ok(())
}
fn decode_parts(
    bank: &Bank,
    parts: &[PartEntry],
    count: usize,
) -> Result<Vec<Vec<SampleWords>>, String> {
    let hierarchy = bank.hierarchy().ok_or("Missing stock hierarchy")?;
    let bones = hierarchy.bone_count as usize;
    if parts.len() != hierarchy.parts.len() {
        return Err("Animation part count differs from hierarchy".into());
    }
    let mut frames = vec![vec![[0; 10]; bones]; count];
    let mut assigned = vec![false; bones];
    for (entry, layout) in parts.iter().zip(&hierarchy.parts) {
        let part = entry
            .part
            .as_ref()
            .ok_or("Required stock animation part is absent")?;
        let mut decoder = VbrDecoder::new(
            bank.bytes(),
            part.offset,
            part.compression_header_relative as usize,
            part.compressed_data_relative as usize,
        )?;
        if decoder.frame_count() != count || decoder.channel_count() != layout.bone_count as usize {
            return Err(format!(
                "{}: codec frame/channel count disagrees with bank header",
                layout.name
            ));
        }
        let start =
            usize::try_from(layout.sqt_offset).map_err(|_| "Negative animation part offset")?;
        let end = start + layout.bone_count as usize;
        if end > bones || assigned[start..end].iter().any(|b| *b) {
            return Err("Overlapping animation parts".into());
        }
        assigned[start..end].fill(true);
        for (index, frame) in frames.iter_mut().enumerate() {
            frame[start..end].copy_from_slice(&decoder.decode_frame(index)?);
        }
    }
    if assigned.iter().any(|v| !*v) {
        return Err("Incomplete animation hierarchy channels".into());
    }
    for frame in &frames {
        validate_samples(frame, bones)?;
    }
    Ok(frames)
}

/// One stock clip's keys, decoded on first use. `decode_parts` decodes the same
/// words for every key at once.
pub(super) struct StockKeys {
    name: String,
    bank: Arc<Bank>,
    bones: usize,
    parts: Vec<StockPart>,
    count: usize,
    cache: Mutex<Cache>,
}
struct StockPart {
    stream: VbrStream,
    start: usize,
}
/// The decoded keys and each part's unpacked blocks, and the bytes they hold.
struct Cache {
    keys: Box<[Option<Arc<[SampleWords]>>]>,
    blocks: Box<[Box<[Option<Vec<[f32; 8]>>]>]>,
    bytes: usize,
}
impl StockKeys {
    pub(super) fn len(&self) -> usize {
        self.count
    }
    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub(super) fn decoded_bytes(&self) -> usize {
        self.cache().bytes
    }
    /// Drops every decoded key and unpacked block; returns the bytes released.
    /// Keys handed out stay valid until their last handle goes.
    pub(super) fn release(&self) -> usize {
        let mut cache = self.cache();
        cache.keys.iter_mut().for_each(|key| *key = None);
        for part in cache.blocks.iter_mut() {
            part.iter_mut().for_each(|block| *block = None);
        }
        std::mem::take(&mut cache.bytes)
    }
    pub(super) fn get(&self, index: usize) -> Result<Arc<[SampleWords]>, String> {
        if index >= self.count {
            return Err(format!("{}: animation key {index} out of range", self.name));
        }
        let mut cache = self.cache();
        if let Some(key) = &cache.keys[index] {
            return Ok(key.clone());
        }
        let key: Arc<[SampleWords]> = self
            .decode(index, &mut cache)
            .map_err(|e| format!("{}: {e}", self.name))?
            .into();
        cache.bytes += self.bones * size_of::<SampleWords>();
        cache.keys[index] = Some(key.clone());
        Ok(key)
    }
    fn decode(&self, index: usize, cache: &mut Cache) -> Result<Vec<SampleWords>, String> {
        let bytes = self.bank.bytes();
        let mut frame = vec![[0; 10]; self.bones];
        let block = index / 8;
        for (part, blocks) in self.parts.iter().zip(cache.blocks.iter_mut()) {
            if blocks[block].is_none() {
                let words = part.stream.unpack_block(bytes, block)?;
                cache.bytes += words.len() * size_of::<[f32; 8]>();
                blocks[block] = Some(words);
            }
            let coefficients = blocks[block].as_ref().unwrap();
            let end = part.start + part.stream.channel_count();
            part.stream
                .decode_frame_into(bytes, index, coefficients, &mut frame[part.start..end])?;
        }
        validate_samples(&frame, self.bones)?;
        Ok(frame)
    }
}
/// The checks of `decode_parts`, without decoding any key.
fn stock_keys(
    bank: &Arc<Bank>,
    parts: &[PartEntry],
    count: usize,
    name: &str,
) -> Result<StockKeys, String> {
    let hierarchy = bank.hierarchy().ok_or("Missing stock hierarchy")?;
    let bones = hierarchy.bone_count as usize;
    if parts.len() != hierarchy.parts.len() {
        return Err("Animation part count differs from hierarchy".into());
    }
    let mut assigned = vec![false; bones];
    let mut streams = Vec::with_capacity(parts.len());
    for (entry, layout) in parts.iter().zip(&hierarchy.parts) {
        let part = entry
            .part
            .as_ref()
            .ok_or("Required stock animation part is absent")?;
        let stream = VbrStream::new(
            bank.bytes(),
            part.offset,
            part.compression_header_relative as usize,
            part.compressed_data_relative as usize,
        )?;
        if stream.frame_count() != count || stream.channel_count() != layout.bone_count as usize {
            return Err(format!(
                "{}: codec frame/channel count disagrees with bank header",
                layout.name
            ));
        }
        let start =
            usize::try_from(layout.sqt_offset).map_err(|_| "Negative animation part offset")?;
        let end = start + layout.bone_count as usize;
        if end > bones || assigned[start..end].iter().any(|b| *b) {
            return Err("Overlapping animation parts".into());
        }
        assigned[start..end].fill(true);
        streams.push(StockPart { stream, start });
    }
    if assigned.iter().any(|v| !*v) {
        return Err("Incomplete animation hierarchy channels".into());
    }
    let blocks = streams
        .iter()
        .map(|part| (0..part.stream.block_count()).map(|_| None).collect())
        .collect();
    Ok(StockKeys {
        name: name.to_owned(),
        bank: bank.clone(),
        bones,
        parts: streams,
        count,
        cache: Mutex::new(Cache {
            keys: (0..count).map(|_| None).collect(),
            blocks,
            bytes: 0,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the user's stock animation banks (SKATE3_ASSET_ROOT)"]
    fn lazy_keys_equal_whole_clip_decode() {
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("Set SKATE3_ASSET_ROOT");
        let banks = AnimationBanks::load(std::path::Path::new(&root)).unwrap();
        let native = NativeFrames::new(&banks).unwrap();
        let mut keys = 0;
        for name in native.clip_names() {
            let entry = &native.clips[name];
            let bank = &native.banks[entry.bank];
            let record = bank.record_at(entry.offset).unwrap();
            let RecordData::Clip(clip) = &record.data else { unreachable!() };
            let lazy = native.clip(name).unwrap();
            let whole = decode_parts(bank, &clip.parts, lazy.frames.len()).unwrap();
            // Decode in a scattered order; a key must not depend on its predecessors.
            for index in (0..whole.len()).rev().chain(0..whole.len()) {
                assert_eq!(&*lazy.frames.get(index).unwrap(), &whole[index][..], "{name} {index}");
            }
            keys += whole.len();
        }
        assert_eq!(keys, 131_642);
    }

    #[test]
    #[ignore = "requires the user's stock animation banks (SKATE3_ASSET_ROOT)"]
    fn trimmed_keys_decode_again_identically() {
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("Set SKATE3_ASSET_ROOT");
        let banks = AnimationBanks::load(std::path::Path::new(&root)).unwrap();
        let native = NativeFrames::new(&banks).unwrap();
        let names: Vec<String> = native.clip_names().take(40).map(str::to_owned).collect();
        let before: Vec<_> = names.iter().map(|n| native.clip(n).unwrap().frames.to_vec().unwrap()).collect();
        let full = native.trim(usize::MAX);
        assert!(full > 0);
        // Used in this epoch and the last: kept even over budget.
        assert_eq!(native.trim(0), full);
        // Idle for two epochs: released.
        assert_eq!(native.trim(0), 0);
        for (name, words) in names.iter().zip(&before) {
            assert_eq!(&native.clip(name).unwrap().frames.to_vec().unwrap(), words, "{name}");
        }
        assert_eq!(native.trim(usize::MAX), full);
    }
}
