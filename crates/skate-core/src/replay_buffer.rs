//! Instant replay recording of every drawn actor (NPC skaters, peds, cars, props, mod bodies).
//!
//! Retail records presentation data, never a re-simulation: one tracker frame per sim step (60 / s), at most 5400
//! frames in a byte ring that evicts the oldest (DataTracker `82726170` / `82DBBAF8`, b345 / b346). Skater poses are
//! LOCAL joint rotations as 4 x i16 quaternions (scale 32767), interpolated per joint between frames with nlerp
//! (b365 to b367, decoder `8278ABE0`). This buffer keeps one track of keys per actor, keyed by a stable u64 id, and
//! writes a key only when the quantised pose changed, so a resting prop costs one key for the whole window.
//!
//! Engine independent: the caller supplies frame times and poses as plain arrays.
use std::collections::{BTreeMap, VecDeque};

/// Retail frame cap: 90 s at one frame per 60 Hz sim step (`li 5400`, b345).
pub const RETAIL_MAX_FRAMES: u32 = 5400;
/// Retail's byte pool (11,534,336 B, `lis 176`) holds its compressed frames. Ours are larger (uncompressed i16 rows
/// for every drawn joint), so the default budget is ours, sized so a busy scene still holds the retail 5400 frames.
pub const DEFAULT_BYTE_BUDGET: usize = 128 * 1024 * 1024;
/// Quaternion component scale (retail `8278A4E0`: i16 x 32767).
const QUAT_SCALE: f32 = 32767.0;
/// A joint translation within this of its rest value is not stored (local bone offsets are constant up to float
/// noise from the presenters' matrix inverses).
const REST_EPSILON: f32 = 1.0e-4;
/// Eviction runs once this many frames are past the window (and whenever the byte budget is exceeded).
const TRIM_EVERY: usize = 60;

/// Limits, data driven (`replay.max_frames`, `replay.byte_budget`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplayLimits {
    pub max_frames: u32,
    pub byte_budget: usize,
}

impl Default for ReplayLimits {
    fn default() -> Self {
        Self { max_frames: RETAIL_MAX_FRAMES, byte_budget: DEFAULT_BYTE_BUDGET }
    }
}

/// One local transform: translation, rotation quaternion `[x, y, z, w]`, scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointPose {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for JointPose {
    fn default() -> Self {
        Self { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
    }
}

/// One actor as drawn this frame: its root transform and its joints' local transforms, in a fixed order per actor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActorPose {
    pub root: JointPose,
    pub joints: Vec<JointPose>,
    pub hidden: bool,
}

struct Key {
    time: f64,
    data: Vec<u8>,
}

#[derive(Default)]
struct Track {
    /// Joint translations of the first key; later keys store only the joints that moved off them.
    rest: Vec<[f32; 3]>,
    keys: VecDeque<Key>,
    /// Frame time the actor left the world (despawn); not drawn from then on.
    end: Option<f64>,
}

/// The recording: frame times plus one key track per actor id.
#[derive(Default)]
pub struct ReplayBuffer {
    pub limits: ReplayLimits,
    frames: VecDeque<f64>,
    tracks: BTreeMap<u64, Track>,
    bytes: usize,
    since_trim: usize,
}

impl ReplayBuffer {
    pub fn new(limits: ReplayLimits) -> Self {
        Self { limits, ..Self::default() }
    }

    pub fn clear(&mut self) {
        *self = Self::new(self.limits);
    }

    /// Start a recorder frame at `time` (seconds on the replay clock). A clock that went back clears the recording.
    pub fn begin_frame(&mut self, time: f64) {
        match self.frames.back() {
            Some(&last) if time < last => self.clear(),
            Some(&last) if time == last => return,
            _ => {}
        }
        self.frames.push_back(time);
        let max = self.limits.max_frames.max(1) as usize;
        while self.frames.len() > max {
            self.frames.pop_front();
            self.since_trim += 1;
        }
        if self.since_trim >= TRIM_EVERY {
            self.trim();
        }
        while self.bytes > self.limits.byte_budget && self.frames.len() > 1 {
            for _ in 0..TRIM_EVERY.min(self.frames.len() - 1) {
                self.frames.pop_front();
            }
            self.trim();
        }
    }

    /// Record `pose` for `id` in the current frame. Skipped when it quantises to the last key.
    pub fn record(&mut self, id: u64, pose: &ActorPose) {
        let Some(&now) = self.frames.back() else { return };
        let previous_frame = self.frames.len().checked_sub(2).map(|i| self.frames[i]);
        let track = self.tracks.entry(id).or_default();
        track.end = None;
        if track.keys.is_empty() || track.rest.len() != pose.joints.len() {
            if !track.keys.is_empty() {
                // A new skeleton (look reloaded): start the track over.
                self.bytes -= track.bytes();
                track.keys.clear();
            }
            track.rest = pose.joints.iter().map(|j| j.translation).collect();
            self.bytes += track.rest.len() * 12;
        }
        let data = encode(pose, &track.rest);
        let Some(last) = track.keys.back() else {
            self.bytes += data.len() + KEY_OVERHEAD;
            track.keys.push_back(Key { time: now, data });
            return;
        };
        if last.data == data || last.time == now {
            return;
        }
        // Held since an earlier frame: repeat the held pose at the previous frame so playback holds instead of
        // blending across the gap.
        if let Some(previous) = previous_frame.filter(|&t| t > last.time) {
            let held = last.data.clone();
            self.bytes += held.len() + KEY_OVERHEAD;
            track.keys.push_back(Key { time: previous, data: held });
        }
        self.bytes += data.len() + KEY_OVERHEAD;
        track.keys.push_back(Key { time: now, data });
    }

    /// The actor left the world in the current frame.
    pub fn end(&mut self, id: u64) {
        if let (Some(&now), Some(track)) = (self.frames.back(), self.tracks.get_mut(&id)) {
            track.end.get_or_insert(now);
        }
    }

    /// First and last frame time held.
    pub fn bounds(&self) -> Option<(f64, f64)> {
        Some((*self.frames.front()?, *self.frames.back()?))
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Whether `id` still has a track in the window.
    pub fn contains(&self, id: u64) -> bool {
        self.tracks.contains_key(&id)
    }

    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.tracks.keys().copied()
    }

    /// The pose of `id` at `time`: None before its first key or from its despawn on. Between two keys every joint is
    /// blended (translation / scale lerp, rotation nlerp, as retail's decoder); a joint count change does not blend.
    pub fn sample(&self, id: u64, time: f64) -> Option<ActorPose> {
        let track = self.tracks.get(&id)?;
        if track.end.is_some_and(|end| time >= end) {
            return None;
        }
        let upper = track.keys.partition_point(|k| k.time <= time);
        let a = track.keys.get(upper.checked_sub(1)?)?;
        let a_pose = decode(&a.data, &track.rest)?;
        let Some(b) = track.keys.get(upper) else { return Some(a_pose) };
        let b_pose = decode(&b.data, &track.rest)?;
        if b.time <= a.time || a_pose.joints.len() != b_pose.joints.len() {
            return Some(a_pose);
        }
        let alpha = ((time - a.time) / (b.time - a.time)).clamp(0.0, 1.0) as f32;
        Some(ActorPose {
            root: blend(&a_pose.root, &b_pose.root, alpha),
            joints: a_pose.joints.iter().zip(&b_pose.joints).map(|(x, y)| blend(x, y, alpha)).collect(),
            hidden: a_pose.hidden,
        })
    }

    /// Drop keys before the window start (keeping the one that holds at the start) and actors gone before it.
    fn trim(&mut self) {
        self.since_trim = 0;
        let Some(&start) = self.frames.front() else { return };
        let mut freed = 0;
        self.tracks.retain(|_, track| {
            if track.end.is_some_and(|end| end <= start) {
                freed += track.bytes();
                return false;
            }
            while track.keys.len() >= 2 && track.keys[1].time <= start {
                freed += track.keys.pop_front().map_or(0, |k| k.data.len() + KEY_OVERHEAD);
            }
            true
        });
        self.bytes -= freed;
    }
}

/// Bookkeeping per key (time + vector header), counted against the budget.
const KEY_OVERHEAD: usize = 32;

impl Track {
    fn bytes(&self) -> usize {
        self.rest.len() * 12 + self.keys.iter().map(|k| k.data.len() + KEY_OVERHEAD).sum::<usize>()
    }
}

const FLAG_SCALE: u8 = 1;
const FLAG_HIDDEN: u8 = 2;

/// Key layout: flags u8, joint count u16, root (translation 3 x f32, rotation 4 x i16, scale 3 x f32), a bit mask of
/// joints whose translation left the rest value, those translations (3 x f32), every joint's rotation (4 x i16), and
/// with FLAG_SCALE every joint's scale (3 x f32).
fn encode(pose: &ActorPose, rest: &[[f32; 3]]) -> Vec<u8> {
    let scaled = pose.joints.iter().any(|j| j.scale.iter().any(|s| (s - 1.0).abs() > 1.0e-4));
    let n = pose.joints.len();
    let mut out = Vec::with_capacity(3 + 40 + n.div_ceil(8) + n * 8);
    out.push(if scaled { FLAG_SCALE } else { 0 } | if pose.hidden { FLAG_HIDDEN } else { 0 });
    out.extend_from_slice(&(n as u16).to_le_bytes());
    put_f32s(&mut out, &pose.root.translation);
    put_quat(&mut out, pose.root.rotation);
    put_f32s(&mut out, &pose.root.scale);
    let moved: Vec<bool> = pose
        .joints
        .iter()
        .zip(rest)
        .map(|(j, r)| j.translation.iter().zip(r).any(|(a, b)| (a - b).abs() > REST_EPSILON))
        .collect();
    let mut mask = vec![0u8; n.div_ceil(8)];
    for (i, _) in moved.iter().enumerate().filter(|(_, m)| **m) {
        mask[i / 8] |= 1 << (i % 8);
    }
    out.extend_from_slice(&mask);
    for (joint, _) in pose.joints.iter().zip(&moved).filter(|(_, m)| **m) {
        put_f32s(&mut out, &joint.translation);
    }
    for joint in &pose.joints {
        put_quat(&mut out, joint.rotation);
    }
    if scaled {
        for joint in &pose.joints {
            put_f32s(&mut out, &joint.scale);
        }
    }
    out
}

fn decode(data: &[u8], rest: &[[f32; 3]]) -> Option<ActorPose> {
    let mut r = Reader(data);
    let flags = r.u8()?;
    let n = u16::from_le_bytes([r.u8()?, r.u8()?]) as usize;
    if n != rest.len() {
        return None;
    }
    let root = JointPose { translation: r.f32s()?, rotation: r.quat()?, scale: r.f32s()? };
    let mask = r.take(n.div_ceil(8))?.to_vec();
    let mut joints: Vec<JointPose> = rest.iter().map(|t| JointPose { translation: *t, ..JointPose::default() }).collect();
    for (i, joint) in joints.iter_mut().enumerate() {
        if mask[i / 8] & (1 << (i % 8)) != 0 {
            joint.translation = r.f32s()?;
        }
    }
    for joint in &mut joints {
        joint.rotation = r.quat()?;
    }
    if flags & FLAG_SCALE != 0 {
        for joint in &mut joints {
            joint.scale = r.f32s()?;
        }
    }
    Some(ActorPose { root, joints, hidden: flags & FLAG_HIDDEN != 0 })
}

fn put_f32s(out: &mut Vec<u8>, v: &[f32; 3]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// Normalised, w kept non-negative (q and -q are the same rotation), each component x 32767.
fn put_quat(out: &mut Vec<u8>, q: [f32; 4]) {
    let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    let q = if len > 1.0e-12 && len.is_finite() { q.map(|c| c / len) } else { [0.0, 0.0, 0.0, 1.0] };
    let sign = if q[3] < 0.0 { -1.0 } else { 1.0 };
    for c in q {
        out.extend_from_slice(&((c * sign * QUAT_SCALE).round().clamp(-QUAT_SCALE, QUAT_SCALE) as i16).to_le_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn f32s(&mut self) -> Option<[f32; 3]> {
        let b = self.take(12)?;
        Some(std::array::from_fn(|i| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())))
    }
    fn quat(&mut self) -> Option<[f32; 4]> {
        let b = self.take(8)?;
        Some(normalise(std::array::from_fn(|i| i16::from_le_bytes([b[i * 2], b[i * 2 + 1]]) as f32 / QUAT_SCALE)))
    }
}

fn normalise(q: [f32; 4]) -> [f32; 4] {
    let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if len > 1.0e-12 { q.map(|c| c / len) } else { [0.0, 0.0, 0.0, 1.0] }
}

fn lerp3(a: &[f32; 3], b: &[f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// Per joint: translation / scale lerp, rotation nlerp along the shorter arc.
fn blend(a: &JointPose, b: &JointPose, t: f32) -> JointPose {
    let dot: f32 = a.rotation.iter().zip(&b.rotation).map(|(x, y)| x * y).sum();
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    JointPose {
        translation: lerp3(&a.translation, &b.translation, t),
        rotation: normalise(std::array::from_fn(|i| a.rotation[i] + (b.rotation[i] * sign - a.rotation[i]) * t)),
        scale: lerp3(&a.scale, &b.scale, t),
    }
}

#[cfg(test)]
mod tests;
