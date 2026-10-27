//! The Hall of Meat bail injuries (retail tracker 0x82DAF1D8, impacts 0x82DAE9A8 / 0x82DAE6F0, counts 0x82510990).
//! Tuning is `scoring_wipeouts/default` (BoneSet, impact scale); details: docs/hails-additions/31-retail-menus.md#career-stats-2026-10-25-user-asks-334

pub const BONES: usize = 25;
pub const LEVELS: usize = 6;

/// A bone touches the world when its contact record's world flag (+0x73, our `groups[3]`) is set and the channel's
/// normal points up (y over 0.33, 0x82238694); bones 0..22 use part i + 1's normal, 23 / 24 the specific normal of
/// parts 1 / 23 (0x82DAE9A8). The skater is airborne in the bail when no bone touches (M+0x8B8).
pub const CONTACT_NORMAL_Y: f32 = 0.33;

pub fn airborne(world_contact: &[bool; 24], normal_y: &[f32; 24], specific_normal_y: &[f32; 24]) -> bool {
    !(0..BONES).any(|i| {
        let (part, y) = match i {
            23 => (1, specific_normal_y[1]),
            24 => (23, specific_normal_y[23]),
            _ => (i + 1, normal_y[i + 1]),
        };
        world_contact[part] && y > CONTACT_NORMAL_Y
    })
}

/// The impact each bone reads from the skeleton contact record (0x82DAE9A8): bones 0..22 part i + 1's force, bone
/// 23 part 1's specific force, bone 24 part 23's specific force. The record is Fill 0x82BD60C8's copy of the contact
/// array (+0x34 / +0x64 <- force / specific force).
pub fn part_impacts(force: &[f32; 24], specific_force: &[f32; 24]) -> [f32; BONES] {
    std::array::from_fn(|i| match i {
        23 => specific_force[1],
        24 => specific_force[23],
        _ => force[i + 1],
    })
}

/// One BoneSet entry (0x54 bytes): up to 4 links {bone, factor} that an impact spreads to, and 6 levels
/// {points, threshold} (a threshold of 0 turns the level off).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bone {
    pub links: [Option<(usize, f32)>; 4],
    pub levels: [(u32, f32); LEVELS],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    pub bones: [Bone; BONES],
    /// layout+0xBFC (`Hash_B7514DE7EBDE9D01`, retail 0.75); the part's impact float is doubled, then scaled by it.
    pub impact_scale: f32,
    /// An impact (or a spread share) must be above this to count (0x820641A8).
    pub floor: f32,
    /// The accumulator's decay per console tick (0x8208824C).
    pub decay: f32,
}

impl Tuning {
    /// The BoneSet field as big-endian words (25 x 21).
    pub fn from_words(words: &[u32], impact_scale: f32) -> Option<Self> {
        if words.len() != BONES * 21 {
            return None;
        }
        let mut bones = [Bone::default(); BONES];
        for (i, bone) in bones.iter_mut().enumerate() {
            let w = &words[i * 21..(i + 1) * 21];
            for (l, link) in bone.links.iter_mut().enumerate() {
                let id = w[1 + l * 2];
                *link = (id != u32::MAX && (id as usize) < BONES).then(|| (id as usize, f32::from_bits(w[2 + l * 2])));
            }
            for (k, level) in bone.levels.iter_mut().enumerate() {
                *level = (w[9 + k * 2], f32::from_bits(w[10 + k * 2]));
            }
        }
        Some(Self { bones, impact_scale, floor: 0.1, decay: 0.8 })
    }
}

/// One bail's bone state: the accumulator, its peak and the injury level (-1 none, 0..5).
#[derive(Clone, Debug, PartialEq)]
pub struct Injuries {
    accumulated: [f32; BONES],
    peak: [f32; BONES],
    pub level: [i8; BONES],
}

impl Default for Injuries {
    fn default() -> Self {
        Self { accumulated: [0.0; BONES], peak: [0.0; BONES], level: [-1; BONES] }
    }
}

/// The injury counts 0x82510990 makes from the levels (one per bone, at its highest level).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InjuryCounts {
    pub broken: u32,
    pub dislocations: u32,
    pub sprains: u32,
    pub bruises: u32,
    pub scrapes: u32,
}

impl Injuries {
    /// One tick: `part_impacts[i]` is bone i's contact impact float (before scaling); each over the floor adds to the
    /// bone and spreads along its links, then the levels are taken from the peaks and the accumulators decay.
    /// `ticks` is the step in console ticks (1 at 60 Hz) for the decay.
    pub fn tick(&mut self, tuning: &Tuning, part_impacts: &[f32; BONES], ticks: f32) {
        for (i, raw) in part_impacts.iter().enumerate() {
            let impact = raw * 2.0 * tuning.impact_scale;
            if impact > tuning.floor {
                self.add(tuning, i, impact, i);
            }
        }
        for i in 0..BONES {
            self.level[i] = (0..LEVELS)
                .rev()
                .find(|&k| {
                    let threshold = tuning.bones[i].levels[k].1;
                    threshold > 0.0 && self.peak[i] > threshold
                })
                .map_or(-1, |k| k as i8);
            self.accumulated[i] *= tuning.decay.powf(ticks);
        }
    }

    /// 0x82DAE6F0: add to the bone, keep the peak, pass factor x impact to each link but the one it came from.
    fn add(&mut self, tuning: &Tuning, bone: usize, impact: f32, from: usize) {
        self.accumulated[bone] += impact;
        self.peak[bone] = self.peak[bone].max(self.accumulated[bone]);
        for (link, factor) in tuning.bones[bone].links.into_iter().flatten() {
            let share = factor * impact;
            if link != from && share > tuning.floor {
                self.add(tuning, link, share, bone);
            }
        }
    }

    pub fn counts(&self) -> InjuryCounts {
        let mut c = InjuryCounts::default();
        for level in self.level {
            match level {
                0 => c.scrapes += 1,
                1 => c.bruises += 1,
                2 => c.sprains += 1,
                3 => c.dislocations += 1,
                4 | 5 => c.broken += 1,
                _ => {}
            }
        }
        c
    }

    /// Row 4: a level on bone 0 or 23 (the head's two channels); row 5: on bone 24 (groin).
    pub fn head_hit(&self) -> bool {
        self.level[0] >= 0 || self.level[23] >= 0
    }

    pub fn groin_hit(&self) -> bool {
        self.level[24] >= 0
    }
}

/// The total body rotation (M+0x81C, degrees): three trackers turn about the hips' axes (physical part 23; retail
/// `[skater+0x14]` +0x30..+0x6F). Each tick a tracker measures how far its reference row turned about its axis row
/// (0x82DAC9D0; a turn over 90 degrees in one tick counts 0) and adds it up; when the sum starts shrinking after more
/// than 30 degrees it is banked as |sum| and restarts from that tick's turn (0x82DAC930). The total is the banked
/// degrees plus the trackers' open |sum| (0x82DAD640). Research b594 / b596 / b597.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rotation {
    trackers: [(f32, Option<[[f32; 4]; 3]>); 3],
    banked: f32,
    pub total: f32,
}

impl Rotation {
    pub const REVERSAL_DEGREES: f32 = 30.0;

    /// `hips` rows 0..2; `counting` false (body at rest, or retail's cVar2) re-seeds the trackers without counting.
    pub fn tick(&mut self, hips: &[[f32; 4]; 3], counting: bool) {
        use crate::player::wipeout_state::{math, orientation};
        // (axis row, reference row) of the hips per tracker: ids 2, 0, 1 at M+0x70 / 0xC0 / 0x110.
        const ROWS: [(usize, usize); 3] = [(2, 0), (0, 1), (1, 0)];
        for (k, (sum, old)) in self.trackers.iter_mut().enumerate() {
            let previous = old.replace(*hips);
            let Some(previous) = previous.filter(|_| counting) else { continue };
            let (q, r) = (previous[ROWS[k].0], previous[ROWS[k].1]);
            let n = math::normalize(math::cross(hips[ROWS[k].1], q));
            let m = math::normalize(math::cross(q, n));
            let delta = if math::dot(m, r) < 0.0 { 0.0 } else { wrap_pi(orientation::signed_angle(m, r, q)) };
            let before = sum.abs().to_degrees();
            let after = (*sum + delta).abs().to_degrees();
            if after < before && before > Self::REVERSAL_DEGREES {
                self.banked += before;
                *sum = delta;
            } else {
                *sum += delta;
            }
        }
        self.total = self.banked + self.trackers.iter().map(|(s, _)| s.abs().to_degrees()).sum::<f32>();
    }
}

fn wrap_pi(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    if a > PI {
        a - TAU
    } else if a <= -PI {
        a + TAU
    } else {
        a
    }
}

/// One BodyTweak element (`scoring_wipeouts` `Hash_5717CFCCCA59BD47`): the pose slot, its stick direction, the held
/// ticks that count a tweak and its points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pose: usize,
    pub direction: [f32; 2],
    pub frames: u32,
    pub points: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoseTable {
    pub poses: Vec<Pose>,
    /// The gesture stick must be longer than this (`Hash_9C494E9614FA5ED6`, retail 0.6; 0 when missing).
    pub dead_zone: f32,
}

impl PoseTable {
    /// The array items as words {pose, handle, dir x, dir y, frames, points}.
    pub fn from_items(items: &[[u32; 6]], dead_zone: f32) -> Self {
        let poses = items
            .iter()
            .map(|w| Pose { pose: w[0] as usize, direction: [f32::from_bits(w[2]), f32::from_bits(w[3])], frames: w[4], points: w[5] })
            .collect();
        Self { poses, dead_zone }
    }

    /// 0x82DAD728 / 0x8296EA60: the first element within pi / count of the stick, outside the dead zone.
    fn matching(&self, stick: [f32; 2]) -> Option<&Pose> {
        if self.dead_zone * self.dead_zone >= stick[0] * stick[0] + stick[1] * stick[1] {
            return None;
        }
        let width = std::f32::consts::PI / self.poses.len().max(1) as f32;
        self.poses.iter().find(|p| angle_2d(p.direction, stick) < width)
    }
}

fn angle_2d(a: [f32; 2], b: [f32; 2]) -> f32 {
    let (la, lb) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1]);
    if !(la > 1e-4 && lb > 1e-4) {
        return 0.0;
    }
    ((a[0] * b[0] + a[1] * b[1]) / (la.sqrt() * lb.sqrt())).clamp(-1.0, 1.0).acos()
}

/// The bail's body tweak poses (air ticks only): counts, rotation, time and longest hold per pose slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Poses {
    current: Option<usize>,
    last_air: Option<usize>,
    held: u32,
    rotation_mark: f32,
    pub rotation: [f32; 4],
    pub time: [f32; 4],
    pub longest: [f32; 4],
    pub tweaks: u32,
    pub points: u32,
}

impl Poses {
    pub const JUDO_POSE: usize = 2;

    /// An air starts (0x82DAF1D8: the matched pose -1, held 0).
    pub fn air_start(&mut self) {
        self.current = None;
        self.last_air = None;
        self.held = 0;
    }

    /// One air tick (0x82DAD728). `total` is the body rotation before this tick's trackers; `frozen` is retail's
    /// cVar2, which holds the counters (a hold frozen at exactly the element's frames adds its tweak every tick, as
    /// retail does). `dt` replaces the 1/60 s tick.
    pub fn air_tick(&mut self, table: &PoseTable, stick: [f32; 2], total: f32, frozen: bool, dt: f32) {
        let previous = self.current.take();
        match table.matching(stick) {
            Some(e) => {
                self.current = Some(e.pose);
                let slot = e.pose.min(3);
                if Some(e.pose) == previous {
                    let last = std::mem::replace(&mut self.rotation_mark, total);
                    if !frozen {
                        self.held += 1;
                        self.rotation[slot] += total - last;
                        self.time[slot] += dt;
                        self.longest[slot] = self.longest[slot].max(self.held as f32 * dt);
                    }
                } else {
                    self.rotation_mark = total;
                    self.held = 0;
                }
                if self.held == e.frames {
                    self.points += e.points;
                    self.tweaks += 1;
                }
            }
            None => self.held = 0,
        }
        self.last_air = self.current;
    }

    /// New hits count as judo kicks while the pose (this air's, or the one the last air ended in) is the judo kick.
    pub fn judo(&self) -> bool {
        self.current == Some(Self::JUDO_POSE) || self.last_air == Some(Self::JUDO_POSE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> Tuning {
        let mut words = vec![0u32; BONES * 21];
        for i in 0..BONES {
            let w = &mut words[i * 21..(i + 1) * 21];
            w[0] = i as u32;
            for l in 0..4 {
                w[1 + l * 2] = u32::MAX;
            }
        }
        // Bone 0: links to 1 (x0.5); thresholds 7 / 14 / off / off / 21.
        words[1] = 1;
        words[2] = 0.5f32.to_bits();
        for (k, t) in [(0, 7.0f32), (1, 14.0), (4, 21.0)] {
            words[10 + k * 2] = t.to_bits();
        }
        // Bone 1: links back to 0 (never followed back) and a threshold 2 at level 0.
        words[21 + 1] = 0;
        words[21 + 2] = 1.0f32.to_bits();
        words[21 + 10] = 2.0f32.to_bits();
        Tuning::from_words(&words, 0.75).unwrap()
    }

    #[test]
    fn impacts_spread_rise_to_the_peak_level_and_decay() {
        let t = tuning();
        let mut s = Injuries::default();
        let mut hit = [0.0; BONES];
        hit[0] = 6.0; // x2 x0.75 = 9
        s.tick(&t, &hit, 1.0);
        assert_eq!((s.level[0], s.level[1]), (0, 0)); // 9 > 7; bone 1 gets 4.5 > 2
        s.tick(&t, &hit, 1.0); // 9 x 0.8 + 9 = 16.2 > 14
        assert_eq!(s.level[0], 1);
        for _ in 0..50 {
            s.tick(&t, &[0.0; BONES], 1.0);
        }
        assert_eq!(s.level[0], 1, "the peak keeps the level");
        assert!(s.head_hit() && !s.groin_hit());
        assert_eq!(s.counts(), InjuryCounts { bruises: 1, scrapes: 1, ..Default::default() });
    }

    #[test]
    fn a_small_impact_counts_nothing() {
        let mut s = Injuries::default();
        let mut hit = [0.0; BONES];
        hit[0] = 0.05; // 0.075 under the floor
        s.tick(&tuning(), &hit, 1.0);
        assert_eq!(s, Injuries { accumulated: [0.0; BONES], peak: [0.0; BONES], level: [-1; BONES] });
    }

    fn table() -> PoseTable {
        let item = |pose: u32, x: f32, y: f32| [pose, 0, x.to_bits(), y.to_bits(), 15, 500];
        PoseTable::from_items(&[item(0, 0.0, 1.0), item(2, 1.0, 0.0), item(1, 0.0, -1.0), item(3, -1.0, 0.0)], 0.6)
    }

    #[test]
    fn a_pose_held_its_frames_counts_one_tweak_and_its_rotation() {
        let t = table();
        let mut p = Poses::default();
        p.air_start();
        // Stick right (pose 2) for 17 ticks while the body turns 10 degrees a tick.
        for i in 0..17 {
            p.air_tick(&t, [0.9, 0.1], i as f32 * 10.0, false, 1.0 / 60.0);
        }
        assert_eq!((p.tweaks, p.points), (1, 500));
        assert!((p.rotation[2] - 160.0).abs() < 1e-3 && p.judo());
        // Inside the dead zone: no pose, the hold resets.
        p.air_tick(&t, [0.3, 0.3], 170.0, false, 1.0 / 60.0);
        assert!(!p.judo() && p.held == 0);
    }

    #[test]
    fn a_barrel_roll_turns_the_total_rotation() {
        let mut r = Rotation::default();
        let frame = |a: f32| [[1.0, 0.0, 0.0, 0.0], [0.0, a.cos(), a.sin(), 0.0], [0.0, -a.sin(), a.cos(), 0.0]];
        for i in 0..=36 {
            r.tick(&frame((i as f32 * 10.0).to_radians()), true);
        }
        assert!((r.total - 360.0).abs() < 1.0, "{}", r.total);
    }
}
