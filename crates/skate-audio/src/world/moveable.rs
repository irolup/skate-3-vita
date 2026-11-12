//! The pushed prop's sound object (`SFXObj_DynamicObject`'s moveable twin, process `sub_824E48D0`, update
//! `sub_824E5F18`; MixMap slot 8 object `Moving`): the start loop (`c_sliding_start` / `c_rolling_start`, bank
//! DMO_Starts), the main loop on the prop's own bank (`c_moveable_sliding_objects` / `c_moveable_rolling_objects`),
//! the manager's pitch judder (`sub_824F5EB8`) and the one-shot interval (`sub_824E6FE8`). Research b630, b632, b639,
//! b640: docs/hails-additions/27-dynamic-props.md#prop-sounds
use crate::player::Outputs;
use crate::world::props::KMH;

pub const SLIDING_START: &str = "c_sliding_start";
pub const ROLLING_START: &str = "c_rolling_start";
pub const SLIDING_MAIN: &str = "c_moveable_sliding_objects";
pub const ROLLING_MAIN: &str = "c_moveable_rolling_objects";
/// The start loops' bank.
pub const START_BANK: &str = "DMO_Starts";
pub const START_WORDS: usize = 9;
pub const SLIDING_WORDS: usize = 50;
pub const ROLLING_WORDS: usize = 54;
/// Fields of each record set (sub_824E6068: obj 0x110.. sliding, 0x1A4.. rolling).
pub const SET: usize = 37;
/// `0x822F8B38`.
const PITCH_SCALE: f32 = f32::from_bits(0xB980_0000);
/// `0x822F8EA4`: the angular word's full scale.
const ANGULAR: f32 = 1024.0;

/// What the moveable object reads from a prop SFX record (`sub_824E6068`).
#[derive(Clone, Debug, PartialEq)]
pub struct MoveableRecord {
    /// `0x48`: the main bank (stem).
    pub bank: String,
    /// `0x13C` patch, `0x134` loop max km/h, `0x138` moved max km/h, `0xB0` pitch scale (per 4096).
    pub patch: i32,
    pub max_kmh: f32,
    pub moved_max_kmh: f32,
    pub pitch_scale: i32,
    /// The sliding set (record 0x124, 0x120, 0xF8, ...; index 35 = the start gain 0x50) and the rolling set (37
    /// optional attributes; index 35 = the rolling start gain 9BA3BB6B50E9C63A).
    pub sliding: [i32; SET],
    pub rolling: [i32; SET],
}

impl Default for MoveableRecord {
    fn default() -> Self {
        Self { bank: String::new(), patch: 0, max_kmh: 1.0, moved_max_kmh: 1.0, pitch_scale: 0, sliding: [0; SET], rolling: [0; SET] }
    }
}

/// `clamp(v * 3.6 / max, 0, 1) * scale` truncated (the speed word 1000, the angular word 1024).
fn word(v: f32, max: f32, scale: f32) -> i32 {
    let mut f = v * KMH / max;
    if !(-f < 0.0) {
        f = 0.0;
    }
    if 1.0 - f < 0.0 {
        f = 1.0;
    }
    (f * scale) as i32
}

/// The pitch word: `p - trunc((scale - m) * p * -1/4096)`, clamped 0..0x2000.
fn pitch_word(p: i32, scale: i32, m: i32) -> i32 {
    (p - ((((scale - m) as f32) * (p as f32)) * PITCH_SCALE) as i32).clamp(0, 0x2000)
}

/// `sub_824B5058` / `sub_824B5148`: the 9-word start packet.
pub fn start_packet(r: &MoveableRecord) -> [i32; START_WORDS] {
    [0x7FFF, 0, 0x1000, 0, 25000, 0, 0, 0, r.patch.clamp(0, 1000)]
}

/// `sub_824B4380` / `sub_824B49D0`: the main packet (50 words sliding, 54 rolling). Each post carries the other set's
/// index-28 field beside its own (retail, b630) and skips index 35 (the start gain).
pub fn main_packet(sliding: bool, speed: i32, owner_b0: f32, r: &MoveableRecord) -> Vec<i32> {
    let (own, other) = if sliding { (&r.sliding, &r.rolling) } else { (&r.rolling, &r.sliding) };
    let mut w = vec![0x7FFF, 0, 0x1000, speed.clamp(0, 1000), 25000, 0, 0, 0, r.patch.clamp(0, 1000), 0, 0, 0];
    if sliding {
        w.push(0x1000);
    } else {
        w[11] = ((owner_b0 * ANGULAR) as i32).clamp(0, 0x400);
        w.extend([0, 0, 0, 0, 0x1000]);
    }
    w.extend(own[..4].iter().map(|v| (*v).clamp(0, 0x400)));
    w.push(own[4].clamp(0, 0x7FFF));
    w.push(own[5].clamp(0, 0x1000));
    w.push(own[6].clamp(0, 0x7FFF));
    let tail: Vec<i32> = if sliding {
        own[7..28].iter().copied().chain([other[28], own[28]]).collect()
    } else {
        own[7..29].iter().copied().chain([other[28]]).collect()
    };
    w.extend(tail.iter().map(|v| (*v).clamp(0, 10000)));
    w.extend(own[29..35].iter().chain([&own[36]]).map(|v| (*v).clamp(0, 0x7FFF)));
    debug_assert_eq!(w.len(), if sliding { SLIDING_WORDS } else { ROLLING_WORDS });
    w
}

/// One evaluation's owner values the moveable words read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveInput {
    /// Owner `+0x54` (the smoothed speed) and `+0x70` (spin).
    pub speed: f32,
    pub spin: f32,
    /// Owner `+0x98` (the moved level), `+0xB0` (entry +0x50).
    pub level: f32,
    pub b0: f32,
    /// The manager's judder `+0x2A4` and its record pitch `+0x2B0`.
    pub judder: i32,
    pub second_pitch: i32,
}

/// `sub_824E4FB0` / `sub_824E58A0`: the start loop's words (outputs 0..5, rolling 14 / 16).
pub fn start_update(w: &mut [i32; START_WORDS], sliding: bool, r: &MoveableRecord, out: &dyn Outputs) {
    w[1] = out.raw(0).clamp(0, 0xFFFF);
    w[2] = pitch_word(out.pitch(2), r.pitch_scale, 0);
    w[4] = out.level(4).clamp(0, 25001);
    w[5] = out.level(3).clamp(0, 25001);
    let (b, a, gain) = if sliding { (5, 1, r.sliding[35]) } else { (16, 14, r.rolling[35]) };
    w[6] = out.level(b).clamp(0, 0x7FFF);
    w[7] = (((out.level(a) as f32) * gain as f32 / 32767.0) as i32).clamp(0, 0x7FFF);
}

/// `sub_824E5208` / `sub_824E5AF8`: the main loop's words (Moving outputs 6..11, rolling 15 / 17).
pub fn main_update(w: &mut [i32], sliding: bool, r: &MoveableRecord, i: &MoveInput, out: &dyn Outputs) {
    let p = out.pitch(8);
    w[1] = out.raw(6).clamp(0, 0xFFFF);
    w[2] = pitch_word(p, r.pitch_scale, i.judder);
    w[3] = word(i.speed, r.max_kmh, 1000.0).clamp(0, 1000);
    w[4] = out.level(10).clamp(0, 25001);
    w[5] = out.level(9).clamp(0, 25001);
    let (level_a, level_b) = if sliding { (7, 11) } else { (15, 17) };
    w[6] = out.level(level_b).clamp(0, 0x7FFF);
    w[7] = ((out.level(level_a) as f32 * i.level) as i32).clamp(0, 0x7FFF);
    let angular = word(i.spin, r.moved_max_kmh, ANGULAR);
    if sliding {
        w[9] = angular;
        w[12] = pitch_word(p, r.pitch_scale, i.second_pitch + i.judder);
    } else {
        w[10] = angular;
        w[11] = ((i.b0 * ANGULAR) as i32).clamp(0, 0x400);
        w[16] = pitch_word(p, r.pitch_scale, i.second_pitch + i.judder);
    }
}

/// The manager's judder ranges (DCD364 record via the prop SFX record's RefSpec; b639 table).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JudderRecord {
    /// km/h low / high of the speed interpolation (`+0x278` / `+0x27C`).
    pub speed: [f32; 2],
    /// The "on" value range (`+0x280` / `+0x284`), "on" ms (`+0x298` / `+0x29C`), "off" ms at low / high speed.
    pub on_value: [i32; 2],
    pub on_ms: [i32; 2],
    pub off_low_ms: [i32; 2],
    pub off_high_ms: [i32; 2],
    /// `+0x2B0`: the second pitch term.
    pub pitch: i32,
}

/// `sub_824F5EB8`: the manager's random pitch judder (one per manager, i.e. per moved prop).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Judder {
    /// `+0x2A4`, `+0x2A0` timer (s), `+0x2A8` phase.
    pub value: i32,
    timer: f32,
    on: bool,
}

impl Default for Judder {
    /// After a record key change (`sub_824F4AD8`): value 0x1000, timer 0, the "on" branch next.
    fn default() -> Self {
        Self { value: 0x1000, timer: 0.0, on: false }
    }
}

/// `lo + rand % (hi - lo)` (unsigned), `lo` when the range is empty.
fn pick(lo: i32, hi: i32, rand: &mut dyn FnMut() -> u32) -> i32 {
    if lo < hi { lo.wrapping_add((rand() % (hi - lo) as u32) as i32) } else { lo }
}

impl Judder {
    /// One tick of `dt` seconds at `kmh` (the manager's +0x50).
    pub fn tick(&mut self, r: &JudderRecord, dt: f32, kmh: f32, rand: &mut dyn FnMut() -> u32) {
        self.timer -= dt;
        if !(self.timer < 0.0) {
            return;
        }
        let ms = if !self.on {
            self.value = pick(r.on_value[0], r.on_value[1], rand);
            pick(r.on_ms[0], r.on_ms[1], rand)
        } else {
            self.value = 0;
            let (mut lo, mut hi) = (r.off_low_ms[0], r.off_low_ms[1]);
            let speed_hi = if r.speed[1] > r.speed[0] { r.speed[1] } else { r.speed[0] + 1.0 };
            if kmh >= speed_hi {
                (lo, hi) = (r.off_high_ms[0], r.off_high_ms[1]);
            } else if r.speed[0] < kmh {
                let t = (kmh - r.speed[0]) / (speed_hi - r.speed[0]);
                lo += ((r.off_high_ms[0] - r.off_low_ms[0]) as f32 * t) as i32;
                hi += ((r.off_high_ms[1] - r.off_low_ms[1]) as f32 * t) as i32;
            }
            pick(lo, hi, rand)
        };
        self.on = !self.on;
        self.timer = ms as f32 * 0.001;
    }
}

/// One DMO type's pushed-prop sound data (its primary prop SFX record and the records it names).
#[derive(Clone, Debug, PartialEq)]
pub struct MovingType {
    pub record: MoveableRecord,
    /// The one-shot record (`None` = no one-shots).
    pub oneshot: Option<OneShotRecord>,
    pub judder: JudderRecord,
    /// The manager's EQ / gain record (the same DCD364 record as the judder).
    pub manager: ManagerRecord,
    /// The record's push / pull frequency shifts (`0xA4` / `0xA8`, Hz).
    pub push_pull: [f32; 2],
}

/// The one-shot layer's record (class 74F2B4132A1D39EA via the prop SFX record's RefSpec at 0x18; b630 section 3).
#[derive(Clone, Debug, PartialEq)]
pub struct OneShotRecord {
    /// The Splice bank (stem) and the sound id (`< 0` = none).
    pub bank: String,
    pub id: i32,
    /// Level and pitch scales, the speed (km/h) of the "full" interval, the intervals (ms) at rest / at full speed, the
    /// chance (%).
    pub level: f32,
    pub pitch: f32,
    pub full_kmh: f32,
    pub rest_ms: [i32; 2],
    pub full_ms: [i32; 2],
    pub chance: i32,
}

/// `sub_824E6FE8`: seconds until the next one-shot at the moved speed.
pub fn oneshot_interval(r: &OneShotRecord, speed: f32, rand: &mut dyn FnMut() -> u32) -> f32 {
    let mut t = speed * KMH / r.full_kmh;
    if !(-t < 0.0) {
        t = 0.0;
    }
    if 1.0 - t < 0.0 {
        t = 1.0;
    }
    let lerp = |a: i32, b: i32| a + ((b - a) as f32 * t) as i32;
    pick(lerp(r.rest_ms[0], r.full_ms[0]), lerp(r.rest_ms[1], r.full_ms[1]), rand) as f32 * 0.001
}

/// A PI20 #1 LFO block of the manager record (graph A: 0x120.., graph B: 0x68..; research b640 section 4).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LfoRecord {
    pub q: f32,
    pub gain: f32,
    /// km/h low / high, centre Hz at low / high speed (the low one is also the floor).
    pub speed: [f32; 2],
    pub hz: [f32; 2],
    pub period_ms: f32,
    /// The depth wobble's hold (ms) and range (milli-Hz), the base depth (Hz).
    pub wobble_hold: [i32; 2],
    pub wobble: [i32; 2],
    pub depth: f32,
}

/// A PI20 #2 gain jitter block (graph A: 0xE8.., graph B: 0x30..): Q, hold (ms), gain range (milli).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JitterRecord {
    pub q: f32,
    pub hold: [i32; 2],
    pub gain: [i32; 2],
}

/// Graph A PI20 #2's random centre (0x100..0x11C): km/h low / high, max hold (ms) at low / high speed, Hz range
/// (milli-Hz).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FreqRecord {
    pub speed: [f32; 2],
    pub hold_max: [i32; 2],
    pub hz: [i32; 2],
}

/// A Gai0 block (G0: 0x98..0xBC, G1: 0x04..0x28): km/h low / high, holds (ms) and levels (high, low pairs as
/// stored) at low / high speed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GainRecord {
    pub speed: [f32; 2],
    pub hold_low: [i32; 2],
    pub hold_high: [i32; 2],
    pub level_low: [i32; 2],
    pub level_high: [i32; 2],
}

/// The manager record (class DCD364A878BA0AA3 via the prop SFX record's RefSpec at 0x30; the judder part is
/// [`JudderRecord`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ManagerRecord {
    /// [graph A, graph B].
    pub lfo: [LfoRecord; 2],
    pub jitter: [JitterRecord; 2],
    /// `0xFC`: both PI20 #2s' base gain (retail's graph B reads graph A's, b640 quirk).
    pub base_gain: f32,
    pub freq: FreqRecord,
    /// [G0 (graph A), G1 (graph B)].
    pub gains: [GainRecord; 2],
    /// `0x00` / 32767: graph B's gain scale.
    pub b_scale: f32,
}

/// `0x822FD218`: the LFO phase step per tick (ms, fixed: one 60 Hz frame).
const LFO_STEP_MS: f32 = 16.6667;

fn lerp_speed(v: f32, speed: [f32; 2]) -> Option<f32> {
    if v <= speed[0] {
        Some(0.0)
    } else if v > speed[1] {
        Some(1.0)
    } else if speed[1] > speed[0] {
        Some((v - speed[0]) / (speed[1] - speed[0]))
    } else {
        None
    }
}

/// `pick` with the sign of a second draw's low bit (the wobble / jitter values).
fn signed(lo: i32, hi: i32, rand: &mut dyn FnMut() -> u32) -> f32 {
    let v = pick(lo, hi, rand) as f32 * 0.001;
    if rand() & 1 == 1 { -v } else { v }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Lfo {
    phase: f32,
    wobble: f32,
    timer: f32,
    hz: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Jitter {
    timer: f32,
    gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GainState {
    timer: f32,
    value: i32,
}

/// The manager's per-tick EQ and gain modulation (`sub_824F53F0`, `sub_824F5D10`); [`Judder`] is its pitch part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqManager {
    lfo: [Lfo; 2],
    jitter: [Jitter; 2],
    freq: Lfo,
    gains: [GainState; 2],
}

impl Default for EqManager {
    /// After a record key change (`sub_824F4AD8`): phases, timers 0, gain values 0x7FFF.
    fn default() -> Self {
        Self { lfo: [Lfo::default(); 2], jitter: [Jitter::default(); 2], freq: Lfo::default(), gains: [GainState { timer: 0.0, value: 0x7FFF }; 2] }
    }
}

impl EqManager {
    /// One tick at `kmh`: the two graphs' parameters (`angle` = Pn21, `shift` = the last push / pull post).
    pub fn tick(&mut self, r: &ManagerRecord, dt: f32, kmh: f32, angle: f32, shift: f32, rand: &mut dyn FnMut() -> u32) -> [crate::bus::dynamic::DynamicParams; 2] {
        use crate::bus::dynamic::{DynamicParams, Peak};
        for p in 0..2 {
            let (s, rec) = (&mut self.lfo[p], &r.lfo[p]);
            s.phase += LFO_STEP_MS;
            let period = if rec.period_ms <= 0.0 { 500.0 } else { rec.period_ms };
            if s.phase > period {
                s.phase = 0.0;
            }
            let sine = crate::dsp::fss::sin(s.phase / period * std::f32::consts::TAU);
            s.timer -= dt;
            if s.timer <= 0.0 {
                s.wobble = signed(rec.wobble[0], rec.wobble[1], rand);
                s.timer = pick(rec.wobble_hold[0], rec.wobble_hold[1], rand) as f32 * 0.001;
            }
            let depth = (rec.depth + s.wobble).max(0.0);
            let base = lerp_speed(kmh, rec.speed).map_or(rec.hz[0], |t| rec.hz[0] + (rec.hz[1] - rec.hz[0]) * t);
            s.hz = rec.hz[0].max(base + sine * depth * 0.5);
            let (j, jr) = (&mut self.jitter[p], &r.jitter[p]);
            j.timer -= dt;
            if j.timer <= 0.0 {
                j.gain = (r.base_gain + signed(jr.gain[0], jr.gain[1], rand)).clamp(0.1, 20.0);
                j.timer = pick(jr.hold[0], jr.hold[1], rand) as f32 * 0.001;
            }
            // sub_824F59E0 runs after each jitter (twice per tick) but only on graph A's block.
            let f = &mut self.freq;
            f.timer -= dt;
            if f.timer <= 0.0 {
                f.hz = pick(r.freq.hz[0], r.freq.hz[1], rand) as f32 * 0.001;
                let t = lerp_speed(kmh, r.freq.speed).unwrap_or(1.0);
                let n = r.freq.hold_max[0] + ((r.freq.hold_max[1] - r.freq.hold_max[0]) as f32 * t) as i32;
                f.timer = if n < 1 { 0.0 } else { (rand() % n as u32) as f32 * 0.001 };
            }
        }
        for (g, rec) in r.gains.iter().enumerate() {
            let s = &mut self.gains[g];
            s.timer -= dt;
            if s.timer < 0.0 {
                let t = lerp_speed(kmh, rec.speed).unwrap_or(1.0);
                let mix = |lo: i32, hi: i32| lo + ((hi - lo) as f32 * t) as i32;
                let level = [mix(rec.level_low[0], rec.level_high[0]), mix(rec.level_low[1], rec.level_high[1])];
                let hold = [mix(rec.hold_low[0], rec.hold_high[0]), mix(rec.hold_low[1], rec.hold_high[1])];
                s.value = pick(level[1], level[0], rand);
                s.timer = pick(hold[0], hold[1], rand) as f32 * 0.001;
            }
        }
        let gain_a = self.gains[0].value as f32 / 32767.0;
        let gain_b = self.gains[1].value as f32 / 32767.0 * gain_a * r.b_scale;
        let lfo = |p: usize| Peak { freq: self.lfo[p].hz, gain: r.lfo[p].gain, q: r.lfo[p].q };
        [
            DynamicParams { gain: gain_a, shift, peaks: [lfo(0), Peak { freq: self.freq.hz, gain: self.jitter[0].gain, q: r.jitter[0].q }], angle },
            // Graph B's PI20 #2 centre is never written (0: the EQ's frequency floor).
            DynamicParams { gain: gain_b, shift, peaks: [lfo(1), Peak { freq: 0.0, gain: self.jitter[1].gain, q: r.jitter[1].q }], angle },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> MoveableRecord {
        MoveableRecord {
            bank: "Moveable_Metal_Box_Large_2".into(),
            patch: 3,
            max_kmh: 10.0,
            moved_max_kmh: 10.0,
            pitch_scale: -1000,
            sliding: std::array::from_fn(|i| 100 + i as i32),
            rolling: std::array::from_fn(|i| 200 + i as i32),
        }
    }

    #[test]
    fn the_main_packets_have_retail_lengths_and_the_cross_set_words() {
        let r = record();
        let s = main_packet(true, 500, 0.0, &r);
        assert_eq!(s.len(), SLIDING_WORDS);
        assert_eq!((s[8], s[12], s[13]), (3, 0x1000, 100), "patch w9, second pitch w13, the set from w14");
        assert_eq!((s[41], s[42]), (228, 128), "w42 = the rolling set's index 28, w43 = the sliding one's");
        assert_eq!((s[43], s[49]), (129, 136), "w44.. = indices 29..34 and 36 (35, the start gain, skipped)");
        let r2 = main_packet(false, 500, 0.25, &r);
        assert_eq!(r2.len(), ROLLING_WORDS);
        assert_eq!((r2[11], r2[16], r2[17]), (256, 0x1000, 200), "w12 = owner +0xB0 x 1024, second pitch w17");
        assert_eq!((r2[45], r2[46]), (228, 128), "w46 = rolling index 28, w47 = the sliding one's");
    }

    #[test]
    fn the_judder_alternates_a_random_value_and_zero() {
        let r = JudderRecord { speed: [5.0, 15.0], on_value: [100, 200], on_ms: [50, 60], off_low_ms: [500, 600], off_high_ms: [100, 200], pitch: 0 };
        let mut j = Judder::default();
        let mut rand = || 7u32;
        j.tick(&r, 1.0 / 30.0, 0.0, &mut rand);
        assert_eq!(j.value, 107, "the first tick after a key change takes the on branch at once");
        for _ in 0..1 {
            j.tick(&r, 1.0 / 30.0, 0.0, &mut rand);
        }
        assert_eq!(j.value, 107, "held for the on time (57 ms)");
        j.tick(&r, 1.0 / 30.0, 0.0, &mut rand);
        assert_eq!(j.value, 0, "then off");
    }

    #[test]
    fn the_manager_modulates_inside_the_record_ranges() {
        let lfo = LfoRecord { q: 4.0, gain: 0.6, speed: [1.0, 12.0], hz: [500.0, 12000.0], period_ms: 1777.0, wobble_hold: [99, 350], wobble: [250, 1500], depth: 7000.0 };
        let r = ManagerRecord {
            lfo: [lfo, lfo],
            jitter: [JitterRecord { q: 3.0, hold: [50, 450], gain: [100, 300] }; 2],
            base_gain: 0.5,
            freq: FreqRecord { speed: [1.0, 9.0], hold_max: [75, 350], hz: [500_000, 10_000_000] },
            gains: [GainRecord { speed: [2.0, 10.0], hold_low: [450, 1250], hold_high: [150, 450], level_low: [32767, 28000], level_high: [32767, 24000] }; 2],
            b_scale: 1.0,
        };
        let mut m = EqManager::default();
        let mut seed = 1u32;
        let mut rand = || {
            seed = seed.wrapping_mul(214_013).wrapping_add(2_531_011);
            (seed >> 16) & 0x7FFF
        };
        for _ in 0..300 {
            let [a, b] = m.tick(&r, 1.0 / 30.0, 6.0, 10.0, -50.0, &mut rand);
            assert!(a.peaks[0].freq >= 500.0 && a.peaks[0].freq <= 12000.0 + 3500.0 + 750.0, "LFO centre {}", a.peaks[0].freq);
            assert!((0.1..=20.0).contains(&a.peaks[1].gain));
            assert!((500.0..10000.0).contains(&a.peaks[1].freq) || a.peaks[1].freq == 0.0);
            assert_eq!(b.peaks[1].freq, 0.0, "graph B's PI20 #2 centre is never written");
            assert!(a.gain > 0.7 && a.gain <= 1.0 && b.gain <= a.gain, "gains {} {}", a.gain, b.gain);
            assert_eq!((a.shift, a.angle), (-50.0, 10.0));
        }
    }

    #[test]
    fn the_pitch_word_applies_the_scale_less_the_judder() {
        assert_eq!(pitch_word(4096, 0, 0), 4096);
        assert_eq!(pitch_word(4096, -1000, 0), 4096 - 1000);
        assert_eq!(pitch_word(4096, -1000, 200), 4096 - 1200, "the judder lowers the pitch further");
    }
}
