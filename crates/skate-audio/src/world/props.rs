//! Dynamic prop (DMO) impacts, MixMap slot 8 (docs/hails-additions/27-dynamic-props.md#prop-sounds). Written from
//! our reading of the retail code (TU3, reference only; addresses are facts):
//!
//! - [`ContactPeak`]: the per-prop ring `sub_82489848` behind the audio entry writers `sub_827A89B0` /
//!   `sub_827A8AD8`: each frame the strongest contact (closing speed along the normal) and its surface, the
//!   impact = the largest of the last three frames;
//! - [`pick`]: the manager `sub_824F4108`, at most [`OBJECTS`] prop SFX objects (the pushed prop, the hardest hit,
//!   the fastest, then table order);
//! - [`ImpactPoster`]: `SFXObj_DynamicObject`'s impact one-shots `sub_824E3BE0` (+ the attenuation
//!   `sub_824E3B08`) into the collision manager, flagged (the materials' alternate records);
//! - [`PropLoop`]: its held rolling / sliding loop (process `sub_824E2790`, packets `sub_824B3F50` /
//!   `sub_824B3B20`, update `sub_824E2940`), banks [`LOOP_BANKS`].
use crate::player::Outputs;
use crate::player::collision::{CollisionTuning, Message, NO_MATERIAL, NO_TIER};

/// Prop SFX objects at once (`sub_824F4108`; the slot's five instances).
pub const OBJECTS: usize = 5;
/// `0x822F8628`: m/s to km/h.
pub const KMH: f32 = 3.6;
/// `0x820641A8`: the secondary material's speed floor (km/h).
pub const SECONDARY_KMH: f32 = 0.1;
/// `0x821F1790`: the impact fraction's upper clamp.
const FRACTION_MAX: f32 = 5.0;

/// The vault class of the prop SFX records (`sub_824B52A0`).
pub const RECORD_CLASS: &str = "Hash_B05F7C74C63ACFB8";

/// `c_dynamic_rolling_objects` (handle `0x8302EF98`) and `c_dynamic_sliding_objects` (`0x8302EF88`),
/// project `Sk8_AEMS_MoveableObjects.csi`.
pub const ROLLING_CLASS: &str = "c_dynamic_rolling_objects";
pub const SLIDING_CLASS: &str = "c_dynamic_sliding_objects";
/// The banks bound to them: Roll keeps every patch, Slide_A patches up to 15, Slide_B 18 and up (research b629).
pub const LOOP_BANKS: [&str; 3] = ["Dynamics_Objects_Roll", "Dynamics_Objects_Slide_A", "Dynamics_Objects_Slide_B"];
/// Payload words of both loop packets.
pub const LOOP_WORDS: usize = 34;
/// `0x82099280`: the loop's speed floor (m/s).
pub const LOOP_SPEED: f32 = 0.2;
/// `0x822F8B38`: the record's pitch scale per 4096 (negative: the record raises the pitch).
const PITCH_SCALE: f32 = f32::from_bits(0xB980_0000);

/// What the loops read from a prop SFX record (`sub_824E3160`; layout offsets, research b628).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopRecord {
    /// `0x13C`: the banks' patch (w9; sound slots 10 x patch .. +5).
    pub patch: i32,
    /// `0x134`: the speed (km/h) of a full speed word.
    pub max_kmh: f32,
    /// `0x130`: the moved prop's speed rise per evaluation (m/s).
    pub rise: f32,
    /// `0x138`: the spin (km/h-scaled) of a full DynamicObjectPhysics input while moved.
    pub moved_max_kmh: f32,
    /// `0xB0`: the pitch scale (obj+0x48, per 4096).
    pub pitch_scale: i32,
    /// w13..w34 of the sliding packet (record words 0x9C, 0x98, 0x94, 0x90, 0x7C, ...).
    pub sliding: [i32; 22],
    /// w13..w34 of the rolling packet (the record's 22 optional attributes).
    pub rolling: [i32; 22],
}

impl Default for LoopRecord {
    fn default() -> Self {
        Self { patch: 0, max_kmh: 1.0, rise: 0.0, moved_max_kmh: 1.0, pitch_scale: 0, sliding: [0; 22], rolling: [0; 22] }
    }
}

/// `sub_824E3810`: the loop speed word 0..1000 (km/h over the record's maximum).
pub fn speed_word(speed: f32, max_kmh: f32) -> i32 {
    let v = if speed < LOOP_SPEED { 0.0 } else { speed };
    let mut f = v * KMH / max_kmh;
    if !(-f < 0.0) {
        f = 0.0;
    }
    if 1.0 - f < 0.0 {
        f = 1.0;
    }
    (f * 1000.0) as i32
}

/// Which loop a prop holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopKind {
    Rolling,
    Sliding,
}

impl LoopKind {
    pub fn class(self) -> &'static str {
        match self {
            LoopKind::Rolling => ROLLING_CLASS,
            LoopKind::Sliding => SLIDING_CLASS,
        }
    }
}

/// `sub_824B3B20` / `sub_824B3F50`: the 34-word start packet.
pub fn loop_packet(kind: LoopKind, speed: i32, r: &LoopRecord) -> [i32; LOOP_WORDS] {
    let mut w = [0; LOOP_WORDS];
    w[0] = 0x7FFF;
    w[2] = 0x1000;
    w[3] = speed.clamp(0, 1000);
    w[4] = 25000;
    w[8] = r.patch.clamp(0, 1000);
    w[10] = 0x7FFF;
    w[11] = 0x7FFF;
    let tail = match kind {
        LoopKind::Rolling => &r.rolling,
        LoopKind::Sliding => &r.sliding,
    };
    for (i, &v) in tail.iter().enumerate() {
        let max = match i {
            0..=3 => 0x400,
            4 | 6 | 19..=21 => 0x7FFF,
            5 => 0x1000,
            _ => 10000,
        };
        w[12 + i] = v.clamp(0, max);
    }
    w
}

/// One evaluation's inputs of a prop's loop (the owner record, `sub_824B5558`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopInput {
    /// `+0x50`: the speed the gate reads; `+0x50` or (moved) `+0x54`: the speed word's.
    pub speed: f32,
    pub word_speed: f32,
    /// `+0xA4`: in contact (held for 15 evaluations after the last contact).
    pub in_contact: bool,
    /// `+0xA5`: slides (else rolls).
    pub sliding: bool,
    /// The moved state's level scale `+0x9C` (`None` = not moved).
    pub moved: Option<f32>,
}

/// `SFXObj_DynamicObject`'s held loop (obj `+0x2C` sliding / `+0x30` rolling).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropLoop {
    /// `+0x110`: last evaluation's record key (`sub_824E38B8`; 0 = none).
    last: u64,
    held: Option<(LoopKind, LoopRecord, [i32; LOOP_WORDS])>,
}

/// What the host does with the prop's voice.
#[derive(Clone, Debug, PartialEq)]
pub enum LoopCommand {
    Post { class: &'static str, words: [i32; LOOP_WORDS] },
    Release,
}

impl PropLoop {
    pub fn held(&self) -> Option<LoopKind> {
        self.held.as_ref().map(|h| h.0)
    }

    /// `sub_824E2790`'s loop half: start (no loop held, moving, in contact, the same record as the last evaluation)
    /// or release. `record` = the record of the owner's current id with its key (`None` = id 0).
    pub fn process(&mut self, i: &LoopInput, record: Option<(u64, &LoopRecord)>) -> Option<LoopCommand> {
        let key = record.map_or(0, |r| r.0);
        let same = key != 0 && key == self.last;
        self.last = key;
        if LOOP_SPEED < i.speed && same && i.in_contact {
            if self.held.is_some() {
                return None;
            }
            let r = *record?.1;
            let speed = speed_word(i.word_speed, r.max_kmh);
            let kind = if i.sliding { LoopKind::Sliding } else { LoopKind::Rolling };
            if kind == LoopKind::Sliding && speed <= 0 {
                return None;
            }
            let words = loop_packet(kind, speed, &r);
            self.held = Some((kind, r, words));
            return Some(LoopCommand::Post { class: kind.class(), words });
        }
        self.held.take().map(|_| LoopCommand::Release)
    }

    /// `sub_824E2AF8` / `sub_824E2E80`: the held packet's words for this evaluation (from the Dynamic owner's
    /// outputs), to redeliver; `None` = no loop.
    pub fn update(&mut self, i: &LoopInput, out: &dyn Outputs) -> Option<[i32; LOOP_WORDS]> {
        let (kind, r, w) = self.held.as_mut()?;
        w[3] = speed_word(i.word_speed, r.max_kmh).clamp(0, 1000);
        let (a, b) = match kind {
            LoopKind::Sliding => (1, 5),
            LoopKind::Rolling => (8, 9),
        };
        let mut level_a = out.level(a);
        let mut level_b = out.level(b);
        if let Some(scale) = i.moved {
            level_a = (scale * out.level(6) as f32) as i32;
            if *kind == LoopKind::Rolling {
                level_b = (scale * out.level(7) as f32) as i32;
            }
        }
        let pitch = out.pitch(2);
        let pitch = pitch - (((r.pitch_scale as f32) * (pitch as f32)) * PITCH_SCALE) as i32;
        w[1] = out.raw(0).clamp(0, 0xFFFF);
        w[7] = level_a.clamp(0, 0x7FFF);
        w[6] = level_b.clamp(0, 0x7FFF);
        w[2] = pitch.clamp(0, 0x2000);
        w[4] = out.level(4).clamp(0, 25001);
        w[5] = out.level(3).clamp(0, 25001);
        Some(*w)
    }
}

/// One prop SFX record (class [`RECORD_CLASS`], layout offsets) as the impact poster reads it (`sub_824E3930`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropSfxRecord {
    /// The record's collection key (its identity: the owner's `+0x80` / `+0x88`).
    pub key: u64,
    /// `+0x168` material A (143 = none).
    pub material: i32,
    /// `+0xA0` secondary material (143 = none).
    pub secondary: i32,
    /// `+0xAC`: post against the contact's surface (else 143).
    pub use_contact_material: bool,
    /// `+0x128` / `+0x12C`: the impact range (m/s).
    pub impact_min: f32,
    pub impact_max: f32,
    /// `+0x164`: evaluations between posts.
    pub cooldown: i32,
    /// Optional `35BC26C216FBCC33` / `D34C61EB3A1B8335`: below this linear speed the impact scales by speed / limit.
    pub speed_attenuation: Option<f32>,
    /// Optional `6A1E46E04CF26CBB` / `7B05C20A7DB5D09C`: the impact scales by (limit - min(spin, limit)) / limit.
    pub spin_attenuation: Option<f32>,
    /// The loop words (`sub_824E3160`).
    pub looping: LoopRecord,
}

impl Default for PropSfxRecord {
    fn default() -> Self {
        Self {
            key: 0,
            material: NO_MATERIAL,
            secondary: NO_MATERIAL,
            use_contact_material: false,
            impact_min: 0.0,
            impact_max: 1.0,
            cooldown: 0,
            speed_attenuation: None,
            spin_attenuation: None,
            looping: LoopRecord::default(),
        }
    }
}

impl PropSfxRecord {
    /// `sub_824E3B08`: the impact after the record's attenuations.
    pub fn attenuated(&self, impact: f32, speed: f32, spin: f32) -> f32 {
        let mut v = impact;
        if let Some(limit) = self.speed_attenuation
            && speed < limit
            && limit > 0.0
        {
            v *= speed / limit;
        }
        if let Some(limit) = self.spin_attenuation
            && limit > 0.0
        {
            v *= (limit - spin.min(limit)) / limit;
        }
        v
    }
}

/// `sub_82489848`: the last three frames' strongest contacts of one prop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactPeak {
    values: [f32; 3],
    materials: [i32; 3],
    next: usize,
}

impl Default for ContactPeak {
    fn default() -> Self {
        Self { values: [0.0; 3], materials: [NO_MATERIAL; 3], next: 0 }
    }
}

impl ContactPeak {
    /// One frame: its strongest contact (`None` = no contact: value 0, material 0) into the ring, then the
    /// largest of the three and its material (ties keep the older slot index, `sub_827A8AD8`).
    pub fn push(&mut self, contact: Option<(f32, i32)>) -> (f32, i32) {
        let (value, material) = contact.unwrap_or((0.0, 0));
        self.values[self.next] = value;
        self.materials[self.next] = material;
        self.next = (self.next + 1) % 3;
        let mut i = usize::from(self.values[0] < self.values[1]);
        if self.values[i] < self.values[2] {
            i = 2;
        }
        (self.values[i], self.materials[i])
    }
}

/// One prop's audio entry for this evaluation (`sub_827A3FD8` + the owner writer `sub_824B5558`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropEntry {
    /// Stable prop id (the table key; replay and network peers derive the same one).
    pub id: u64,
    /// Owner `+0x30`: the prop's position, the local player's while pushed.
    pub position: [f32; 3],
    /// Owner `+0x40` length: linear speed (m/s).
    pub speed: f32,
    /// Owner `+0x70`: angular speed (rad/s).
    pub spin: f32,
    /// Entry `+0x20`: the local player pushes / moves it.
    pub pushed: bool,
    /// Entry `+0x40` / `+0x48`: [`ContactPeak::push`]'s impact and material.
    pub impact: f32,
    pub contact_material: i32,
}

impl PropEntry {
    /// Owner `+0x50`: the speed, while pushed the larger of linear and angular.
    pub fn owner_speed(&self) -> f32 {
        if self.pushed && self.speed < self.spin { self.spin } else { self.speed }
    }
}

/// `sub_824F4108`: the entries (table order) that hold a prop SFX object this evaluation, at most [`OBJECTS`]:
/// every pushed one, then the hardest impact, then the fastest (both strictly above 0), then the rest in order.
pub fn pick(entries: &[PropEntry]) -> Vec<u64> {
    let quota = entries.len().min(OBJECTS);
    let mut out: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].pushed).collect();
    let pushed = out.last().copied();
    let best = |key: &dyn Fn(&PropEntry) -> f32, out: &[usize], skip: &[Option<usize>]| -> Option<usize> {
        if out.len() >= quota {
            return None;
        }
        let mut found = None;
        let mut top = 0.0;
        for (i, e) in entries.iter().enumerate() {
            if !skip.contains(&Some(i)) && top < key(e) {
                top = key(e);
                found = Some(i);
            }
        }
        found
    };
    let hardest = best(&|e| e.impact, &out, &[pushed]);
    out.extend(hardest);
    let fastest = best(&|e| e.speed, &out, &[pushed, hardest]);
    out.extend(fastest);
    for i in 0..entries.len() {
        if out.len() >= quota {
            break;
        }
        if ![pushed, hardest, fastest].contains(&Some(i)) && !out.contains(&i) {
            out.push(i);
        }
    }
    out.truncate(quota);
    out.into_iter().map(|i| entries[i].id).collect()
}

/// `SFXObj_DynamicObject`'s impact state (`sub_824E3BE0`): the cooldown `+0xFC`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImpactPoster {
    cooldown: i32,
}

impl ImpactPoster {
    /// One evaluation: the collision messages of this entry's impact (none while cooling down or below the record's
    /// floor). Material A against the contact surface, then the tier-2 layer as tier 1 and the secondary material.
    pub fn process(&mut self, rec: &PropSfxRecord, e: &PropEntry, t: &CollisionTuning) -> Vec<Message> {
        if self.cooldown >= 1 {
            self.cooldown -= 1;
            return Vec::new();
        }
        let value = rec.attenuated(e.impact, e.speed, e.spin);
        let min = rec.impact_min;
        if !(min < value) {
            return Vec::new();
        }
        let other = if rec.use_contact_material { e.contact_material } else { NO_MATERIAL };
        let mut f = (value - min) / (rec.impact_max - min);
        if !(-f < 0.0) {
            f = 0.0;
        }
        if FRACTION_MAX - f < 0.0 {
            f = FRACTION_MAX;
        }
        let a = rec.material;
        let band = |m: i32| if m < NO_MATERIAL { t.impact_band_flagged(m, f, true) } else { (NO_TIER, 0.0, 0.0) };
        let level = |m: i32, o: i32, (tier, lo, hi): (i32, f32, f32)| {
            if m < NO_MATERIAL { t.contact_level_flagged(m, o, tier, lo, hi, f, true) } else { 0 }
        };
        let (ba, bb) = (band(a), band(other));
        if ba.0 == NO_TIER && bb.0 == NO_TIER {
            return Vec::new();
        }
        let (la, lb) = (level(a, other, ba), level(other, a, bb));
        let message = |material: [i32; 2], tier: [i32; 2], level: [i32; 2]| Message { material, tier, position: e.position, level, local: true };
        let mut out = vec![message([a, other], [ba.0, bb.0], [la, lb])];
        self.cooldown = rec.cooldown;
        let layer = |tier: i32| if tier == 2 { 1 } else { NO_TIER };
        let (ta, tb) = (layer(ba.0), layer(bb.0));
        if ta != NO_TIER || tb != NO_TIER {
            let la2 = (t.level_scale_flagged(a, true) * la as f32) as i32;
            let lb2 = (t.level_scale_flagged(other, true) * lb as f32) as i32;
            out.push(message([a, other], [ta, tb], [la2, lb2]));
        }
        if rec.secondary != NO_MATERIAL && SECONDARY_KMH < e.owner_speed() * KMH {
            let bs = band(rec.secondary);
            if bs.0 != NO_TIER {
                out.push(message([rec.secondary, NO_MATERIAL], [bs.0, 0], [level(rec.secondary, other, bs), 0]));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::collision::Material;

    struct Flat;
    impl Outputs for Flat {
        fn level(&self, id: usize) -> i32 {
            1000 * id as i32
        }
        fn raw(&self, _: usize) -> i32 {
            0x4000
        }
        fn pitch(&self, _: usize) -> i32 {
            4096
        }
    }

    fn moving(sliding: bool) -> LoopInput {
        LoopInput { speed: 2.0, word_speed: 2.0, in_contact: true, sliding, moved: None }
    }

    #[test]
    fn the_speed_word_is_kmh_over_the_record_maximum() {
        assert_eq!(speed_word(0.19, 10.0), 0, "under the 0.2 m/s floor");
        assert_eq!(speed_word(1.0, 10.0), 359, "3.6 is not exact in f32: 359.99998 truncates, as on the console");
        assert_eq!(speed_word(5.0, 10.0), 1000, "clamped");
    }

    #[test]
    fn a_loop_starts_on_the_second_evaluation_with_the_same_record_and_releases_when_it_stops() {
        let r = LoopRecord { patch: 3, max_kmh: 10.0, ..LoopRecord::default() };
        let mut l = PropLoop::default();
        assert_eq!(l.process(&moving(true), Some((7, &r))), None, "the record must match the last evaluation's");
        let Some(LoopCommand::Post { class, words }) = l.process(&moving(true), Some((7, &r))) else { panic!() };
        assert_eq!((class, words[3], words[8]), (SLIDING_CLASS, 719, 3), "2 m/s = 7.2 km/h of 10 (truncated); patch w9");
        assert_eq!(l.process(&moving(true), Some((7, &r))), None, "held");
        assert_eq!(l.process(&LoopInput { in_contact: false, ..moving(true) }, Some((7, &r))), Some(LoopCommand::Release));
    }

    #[test]
    fn the_update_rewrites_the_mix_words_and_scales_the_pitch() {
        let r = LoopRecord { patch: 5, max_kmh: 10.0, pitch_scale: 1024, ..LoopRecord::default() };
        let mut l = PropLoop::default();
        l.process(&moving(false), Some((1, &r)));
        assert!(matches!(l.process(&moving(false), Some((1, &r))), Some(LoopCommand::Post { class: ROLLING_CLASS, .. })));
        let w = l.update(&moving(false), &Flat).unwrap();
        assert_eq!((w[1], w[7], w[6], w[4], w[5]), (0x4000, 8000, 9000, 4000, 3000), "rolling reads levels 8 / 9");
        assert_eq!(w[2], 4096 + 1024, "4096 + 1024 x 4096 / 4096");
        let w = l.update(&LoopInput { moved: Some(0.5), ..moving(false) }, &Flat).unwrap();
        assert_eq!((w[7], w[6]), (3000, 3500), "moved: levels 6 / 7 by the scale");
    }

    fn entry(id: u64) -> PropEntry {
        PropEntry { id, position: [0.0; 3], speed: 0.0, spin: 0.0, pushed: false, impact: 0.0, contact_material: NO_MATERIAL }
    }

    fn tuning() -> CollisionTuning {
        let mut materials = vec![Material::default(); 143];
        let windows = [1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 11000, 12000, 13000, 14000];
        materials[10] = Material { kind: 0, gain: 32767, windows, bands: [0.6, 1.0, 0.3, 0.05], scale: 0.5, ..Material::default() };
        materials[20] = Material { kind: 0, gain: 32767, windows, bands: [0.8, 1.0, 0.4, 0.1], ..Material::default() };
        CollisionTuning { materials, surface_class: vec![0; 95], surface_eq: Vec::new() }
    }

    #[test]
    fn the_peak_is_the_largest_of_three_frames() {
        let mut p = ContactPeak::default();
        assert_eq!(p.push(Some((0.5, 4))), (0.5, 4));
        assert_eq!(p.push(None), (0.5, 4));
        assert_eq!(p.push(Some((0.2, 7))), (0.5, 4));
        assert_eq!(p.push(None), (0.2, 7), "the 0.5 frame left the ring");
        assert_eq!(p.push(None), (0.2, 7));
        assert_eq!(p.push(None), (0.0, 0), "no contact for three frames: value 0, material 0");
    }

    #[test]
    fn the_pick_is_pushed_hardest_fastest_then_order() {
        let mut e: Vec<PropEntry> = (1..=7).map(entry).collect();
        e[5].pushed = true;
        e[3].impact = 2.0;
        e[1].speed = 4.0;
        assert_eq!(pick(&e), vec![6, 4, 2, 1, 3]);
        assert_eq!(pick(&e[..2]), vec![2, 1], "never more than the entries");
    }

    #[test]
    fn an_impact_posts_once_per_cooldown() {
        let t = tuning();
        let rec = PropSfxRecord { material: 10, use_contact_material: true, impact_min: 0.1, impact_max: 1.0, cooldown: 2, ..PropSfxRecord::default() };
        let mut p = ImpactPoster::default();
        let hit = PropEntry { impact: 0.9, contact_material: 20, ..entry(1) };
        let first = p.process(&rec, &hit, &t);
        assert_eq!(first[0].material, [10, 20]);
        assert_eq!(first[0].tier, [2, 2], "t = 0.89 is over both materials' top bands (0.6, 0.8)");
        assert!(first[0].local, "the prop poster sets the message's +40 byte");
        assert_eq!(first[1].tier, [1, 1], "a tier 2 side layers again as tier 1");
        assert_eq!(first[1].level[0], (0.5 * first[0].level[0] as f32) as i32, "scaled by the window record's +40");
        assert!(p.process(&rec, &hit, &t).is_empty() && p.process(&rec, &hit, &t).is_empty(), "two evaluations of cooldown");
        assert!(!p.process(&rec, &hit, &t).is_empty());
    }

    #[test]
    fn below_the_floor_or_attenuated_nothing_posts() {
        let t = tuning();
        let rec = PropSfxRecord { material: 10, impact_min: 0.3, impact_max: 1.0, speed_attenuation: Some(2.0), ..PropSfxRecord::default() };
        let mut p = ImpactPoster::default();
        assert!(p.process(&rec, &PropEntry { impact: 0.3, ..entry(1) }, &t).is_empty(), "the floor is exclusive");
        let slow = PropEntry { impact: 0.9, speed: 0.5, ..entry(1) };
        assert!(p.process(&rec, &slow, &t).is_empty(), "0.9 x 0.5 / 2 = 0.225 under the floor");
        assert!(!p.process(&rec, &PropEntry { speed: 2.0, ..slow }, &t).is_empty());
    }

    #[test]
    fn the_flagged_poster_reads_the_alternate_bands() {
        let mut t = tuning();
        t.materials[10].alt = Some(crate::player::collision::Records { windows: [0; 14], scale: 0.0, bands: [5.0, 6.0, 4.0, 3.0] });
        let rec = PropSfxRecord { material: 10, impact_min: 0.0, impact_max: 1.0, ..PropSfxRecord::default() };
        let mut p = ImpactPoster::default();
        assert!(p.process(&rec, &PropEntry { impact: 1.0, ..entry(1) }, &t).is_empty(), "t = 1 is under the alternate floor 3");
    }
}
