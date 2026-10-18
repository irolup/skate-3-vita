//! The rebind block of Move Object's held update 82D44A10, the hand points of 82D45D30 and the frame blend
//! 82D46218 / 82D463D8 (research b64 [code, TU3 recomp]; main checked the 0.5 hand factor and 82D43B20):
//! - every tick: the push latch (+1200 bit 0x02, set once |Player+736|^2 > 0.04) and the collision timer +1176
//!   (+1/60 while Player+2484 bit 26 is set, else 0);
//! - the block runs only with the hand flag (Player+2476 bit 21), the timer at most 0.4 s and the latch clear;
//!   path A (a published best record and CanGrabSpline 82E08EE8 on the held record at the grip) refreshes the
//!   record (grip kept, no blend); path B (owner flag 0x40, 82D4D150 mode 1 without the held descriptor, 82E08DB8,
//!   the held record's +196 / +216 non-zero) re-grabs with a new grip, zeroes the anchor velocity and starts a
//!   frame blend only when it already held last tick; anything else loses the hold (+1200 bit 0x20 clears);
//! - 82D45D30: the hand points sit at grip +/- 0.5 x |hand bone 3 - hand bone 7| along the record, clamped to
//!   [0, length];
//! - 82D46218: the blend runs only when the jump D = |target - current| reaches T = threshold x 60, at rate
//!   -T / D per tick; else the frame snaps.
//!
//! Multiplayer: [`RebindState`] and [`FrameBlend`] are plain per-skater data.
use crate::player::offboard::grab_scene::{Record, at_distance};

/// Retail numbers (image constants and collection values); every field is data a mod can override.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RebindTuning {
    /// Collision timer gate, s (0x82181B90, 0.4).
    pub timer_limit: f32,
    /// Push latch threshold on |Player+736|^2 (0x822F92D4, 0.04).
    pub latch_speed_sq: f32,
    /// Hand points: factor on the hand bone distance (0x8209975C, 0.5).
    pub hand_factor: f32,
    /// Minimum squared hand point separation for an edge direction (0x8209BE90, 1e-4).
    pub edge_epsilon_sq: f32,
    /// Frame blend threshold: settings+448 x 60 (0x822F860C); settings+448 is unread (1.0 assumed).
    pub blend_threshold: f32,
}

impl Default for RebindTuning {
    fn default() -> Self {
        Self { timer_limit: 0.4, latch_speed_sq: 0.04, hand_factor: 0.5, edge_epsilon_sq: 1e-4, blend_threshold: 60.0 }
    }
}

/// +1176 and +1200 bit 0x02.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RebindState {
    pub timer: f32,
    pub latched: bool,
}

/// What this tick's rebind block decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rebind {
    /// Path A: refresh the held record, keep the grip.
    Refresh,
    /// Path B: bind the mode-1 candidate with a new grip; `blend` when it held last tick.
    Regrab { blend: bool },
    /// Holding lost.
    Lost,
}

/// The inputs of the rebind block (each a retail test result).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RebindInput {
    /// Player+2476 bit 21.
    pub hand_flag: bool,
    /// Player+2480 bit 22 with Player+2084 / +2104: a published best record exists.
    pub has_best: bool,
    /// 82E08EE8 on the held record at the grip (our `still_holds`); only read with `has_best`.
    pub still_holds: bool,
    /// owner+12836 bit 0x40.
    pub owner_ready: bool,
    /// 82D4D150 mode 1 found a record and 82E08DB8 qualified it.
    pub candidate: bool,
    /// The held record's +196 and +216 are non-zero.
    pub held_fields: bool,
    /// Holding (+1200 bit 0x20) from last tick.
    pub was_holding: bool,
}

impl RebindState {
    /// The per-tick part before the block: the push latch and the collision timer (`step` = 1/60 per retail tick).
    pub fn tick(&mut self, t: &RebindTuning, push_speed_sq: f32, collision_flag: bool, step: f32) {
        if push_speed_sq > t.latch_speed_sq {
            self.latched = true;
        }
        self.timer = if collision_flag { self.timer + step } else { 0.0 };
    }

    /// The block itself (82D44DEC..82D44FD8).
    pub fn decide(&self, t: &RebindTuning, i: &RebindInput) -> Rebind {
        if !i.hand_flag || !(self.timer <= t.timer_limit) || self.latched {
            return Rebind::Lost;
        }
        if i.has_best && i.still_holds {
            return Rebind::Refresh;
        }
        if i.owner_ready && i.candidate && i.held_fields {
            return Rebind::Regrab { blend: i.was_holding };
        }
        Rebind::Lost
    }
}

/// 82D45D30: the two hand points on the record (p+, p-) for the hand bones `a` / `b`; `None` when they are closer
/// than the edge epsilon (retail then falls back to the player's frame row).
pub fn hand_points(record: &Record, grip: f32, a: [f32; 3], b: [f32; 3], t: &RebindTuning) -> Option<[[f32; 3]; 2]> {
    let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    let h = t.hand_factor * d;
    let length = record.scalar(176);
    let at = |s: f32| {
        let p = at_distance(record, s.max(0.0).min(length));
        [p[0], p[1], p[2]]
    };
    let (p, m) = (at(grip + h), at(grip - h));
    let e = (p[0] - m[0]).powi(2) + (p[1] - m[1]).powi(2) + (p[2] - m[2]).powi(2);
    (e > t.edge_epsilon_sq).then_some([p, m])
}

/// The hand IK of the held update: +1200 bit 0x40 (`enabled`) and the weight +1132. 82D46610 (82D469C8..82D46C9C)
/// sets the bit during the grab's enter window ([`super::MoveObjectTuning`] `hand_ik_*`); only 82D444A0 full (a grab
/// or a path B re-grab) clears it, so a re-grab after the window leaves the hands without IK, as in retail. 82D45008
/// moves the weight toward 1 while on, toward 0 while off. Plain per-skater data.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HandIk {
    pub enabled: bool,
    pub weight: f32,
}

impl HandIk {
    /// 82D444A0 full: clear the bit (the weight keeps easing out).
    pub fn begin_grab(&mut self) {
        self.enabled = false;
    }

    /// One tick: the 82D46610 gate at `state_time` (Player+2664), then the 82D45008 weight step.
    pub fn tick(&mut self, t: &super::MoveObjectTuning, state_time: f32) {
        if t.hand_ik_enter > 0.0 && state_time <= t.hand_ik_window && 1.0 - t.hand_ik_curve.evaluate(state_time) > t.hand_ik_threshold {
            self.enabled = true;
        }
        let step = if self.enabled { t.hand_ik_rate } else { -t.hand_ik_rate };
        self.weight = (self.weight + step).clamp(0.0, 1.0);
    }
}

/// +1164 / +1168 / +1200 bit 0x08 and the start frame (+80 position row, hands +464 / +512).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameBlend {
    pub weight: f32,
    pub rate: f32,
    pub active: bool,
    pub start: [f32; 3],
    pub start_hands: [[f32; 3]; 2],
}

impl FrameBlend {
    /// 82D46218: start from the current frame toward `target`.
    pub fn start(&mut self, t: &RebindTuning, current: [f32; 3], current_hands: [[f32; 3]; 2], target: [f32; 3]) {
        self.start = current;
        self.start_hands = current_hands;
        let d = ((target[0] - current[0]).powi(2) + (target[1] - current[1]).powi(2) + (target[2] - current[2]).powi(2)).sqrt();
        let ratio = d / t.blend_threshold;
        if ratio >= 1.0 {
            self.weight = 1.0;
            self.rate = -1.0 / ratio;
            self.active = true;
        } else {
            self.weight = -1.0;
            self.rate = -1.0;
            self.active = false;
        }
    }

    /// 82D463D8: this tick's frame position and hands. Retail interpolates the whole frame (82E0A570, not read);
    /// the position and the hands blend linearly at 1 - w.
    pub fn step(&mut self, target: [f32; 3], target_hands: [[f32; 3]; 2]) -> ([f32; 3], [[f32; 3]; 2]) {
        if self.active {
            self.weight = (self.weight + self.rate).clamp(0.0, 1.0);
            if self.weight < self.rate * 0.001 {
                self.active = false;
                self.weight = -1.0;
            }
        }
        if !self.active {
            return (target, target_hands);
        }
        let w = self.weight;
        let mix = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| w * a[i] + (1.0 - w) * b[i]);
        (mix(self.start, target), [mix(self.start_hands[0], target_hands[0]), mix(self.start_hands[1], target_hands[1])])
    }
}

/// 82D444A0 full with 82D43B20: the anchor seed, the nearest projected point moved `anchor_reach` along the latched
/// push row (+368), height dropped.
pub fn seed_anchor(nearest: [f32; 3], push_row: [f32; 3], anchor_reach: f32) -> [f32; 3] {
    [nearest[0] + push_row[0] * anchor_reach, 0.0, nearest[2] + push_row[2] * anchor_reach]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::offboard::move_object::edge_record;

    #[test]
    fn hand_ik_turns_on_in_the_enter_window_and_stays_until_the_next_grab() {
        let t = crate::player::offboard::move_object::MoveObjectTuning::default();
        let mut ik = HandIk::default();
        // Stock curve: 1 - y passes 0.1 between 0.0969 s (0.0786) and 0.1562 s (0.2929).
        ik.tick(&t, 0.05);
        assert!(!ik.enabled && ik.weight == 0.0);
        ik.tick(&t, 0.15);
        assert!(ik.enabled);
        assert!((ik.weight - 0.2).abs() < 1e-6);
        for k in 0..10 {
            ik.tick(&t, 2.0 + k as f32);
        }
        assert!(ik.enabled && ik.weight == 1.0, "past the window it stays on");
        // A re-grab after the window: off for good, the weight eases out.
        ik.begin_grab();
        ik.tick(&t, 3.0);
        assert!(!ik.enabled && (ik.weight - 0.8).abs() < 1e-6);
        // No enter value (a mod sets 0): never on.
        let off = crate::player::offboard::move_object::MoveObjectTuning { hand_ik_enter: 0.0, ..t };
        let mut ik = HandIk::default();
        ik.tick(&off, 0.5);
        assert!(!ik.enabled);
    }

    #[test]
    fn hand_points_sit_half_the_hand_span_around_the_grip() {
        let t = RebindTuning::default();
        let r = edge_record(7, [0.0, 1.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        let [p, m] = hand_points(&r, 1.0, [0.0, 1.0, 0.4], [0.0, 1.0, -0.4], &t).unwrap();
        assert!((p[0] - 1.4).abs() < 1e-5 && (m[0] - 0.6).abs() < 1e-5, "{p:?} {m:?}");
        // Clamped at the ends.
        let [p, m] = hand_points(&r, 1.9, [0.0, 1.0, 0.4], [0.0, 1.0, -0.4], &t).unwrap();
        assert!((p[0] - 2.0).abs() < 1e-5 && (m[0] - 1.5).abs() < 1e-5);
        // Hands together: no edge.
        assert!(hand_points(&r, 1.0, [0.0; 3], [0.0; 3], &t).is_none());
    }

    #[test]
    fn the_rebind_block_follows_the_retail_gates() {
        let t = RebindTuning::default();
        let mut s = RebindState::default();
        let base = RebindInput { hand_flag: true, has_best: true, still_holds: true, owner_ready: true, candidate: true, held_fields: true, was_holding: true };
        assert_eq!(s.decide(&t, &base), Rebind::Refresh);
        assert_eq!(s.decide(&t, &RebindInput { still_holds: false, ..base }), Rebind::Regrab { blend: true });
        assert_eq!(s.decide(&t, &RebindInput { still_holds: false, was_holding: false, ..base }), Rebind::Regrab { blend: false });
        assert_eq!(s.decide(&t, &RebindInput { still_holds: false, held_fields: false, ..base }), Rebind::Lost);
        assert_eq!(s.decide(&t, &RebindInput { hand_flag: false, ..base }), Rebind::Lost);
        // The collision timer past 0.4 s, then the push latch, block both paths.
        for _ in 0..25 {
            s.tick(&t, 0.0, true, 1.0 / 60.0);
        }
        assert_eq!(s.decide(&t, &base), Rebind::Lost);
        s.tick(&t, 0.0, false, 1.0 / 60.0);
        assert_eq!(s.decide(&t, &base), Rebind::Refresh);
        s.tick(&t, 0.05, false, 1.0 / 60.0);
        assert_eq!(s.decide(&t, &base), Rebind::Lost);
    }

    #[test]
    fn a_short_jump_snaps_and_a_long_one_blends() {
        let t = RebindTuning::default();
        let mut b = FrameBlend::default();
        b.start(&t, [0.0; 3], [[0.0; 3]; 2], [10.0, 0.0, 0.0]);
        assert!(!b.active);
        assert_eq!(b.step([10.0, 0.0, 0.0], [[1.0; 3]; 2]).0, [10.0, 0.0, 0.0]);
        b.start(&t, [0.0; 3], [[0.0; 3]; 2], [120.0, 0.0, 0.0]);
        assert!(b.active && (b.rate + 0.5).abs() < 1e-6);
        let (p, _) = b.step([120.0, 0.0, 0.0], [[0.0; 3]; 2]);
        assert!((p[0] - 60.0).abs() < 1e-3, "{p:?}");
        let (p, _) = b.step([120.0, 0.0, 0.0], [[0.0; 3]; 2]);
        assert!((p[0] - 120.0).abs() < 1e-3);
    }

    #[test]
    fn the_anchor_seed_drops_the_height() {
        assert_eq!(seed_anchor([1.0, 2.0, 3.0], [0.0, 0.0, -1.0], 0.7), [1.0, 0.0, 2.3]);
    }
}
