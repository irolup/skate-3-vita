//! The ActionGraph signals an AI skater's PathController posts so a simulated NPC skater rides
//! into its recorded tricks like a player would (doc 26 "Simulated NPC skaters do their tricks").
//! Retail writes named signals into the same intent map the player's pad listener fills; no pad
//! bits or stick gestures (`.local/research/npc/m7-ai-trick-signals.md`, main checked
//! `sub_824691A0` and the signal addresses).
//!
//! Retail (TU3, evidence only; re-implemented):
//! - anticipation `sub_824691A0` (each tick from the node advance `sub_8246D560`): within 3.0 m of
//!   the line (`pc+828`, `0x82063B08`) it looks up to 60 recorded frames past the cursor for a
//!   start-trick node (`sub_82456550`); an ollie or flip there (category 1 / 2) sets `AnticMag`
//!   1.0 and `AnticAngle` 0, or pi for a nollie (the recorded trick, `sub_82469048`);
//!   `sub_824696B8` posts them (and `Crouch` on a crouched node outside a trick, `Manual` before a
//!   manual) when the skater faces along the line (under pi/4, `0x821A9C28`) [code, the posting
//!   gates partly inferred];
//! - dispatch `sub_8246A2E0`: on the tick the committed cursor reaches the start-trick node it
//!   posts `Trick`, the trick's scorable name, `GestureSpeed`, `TrickHeight` and
//!   `DontMirrorTrick`, all 1.0 (the graph enters the trick on `HasAGIntent <name>`, names
//!   compare case-insensitively); only the newest event node of the tick, none when the cursor
//!   crossed more than 3 nodes (1 right after spawn). That perform path is the ollie / flip one only
//!   (categories 1 / 2, `sub_8246A080`); the other categories keep the trick in the slot and the
//!   per-tick writer posts its timed intents, like a player's held inputs (below, research b113).
//! - trick slot: `pc+832` category (from the recorded trick, table 0x820862A8), `pc+844` trick, `pc+904`
//!   ticks since the start node (0 there, +1 after every writer call); an end node or a failed dispatch
//!   clears the category (0x8246A6D4). Jump table 0x8246A45C: 1 / 2 perform; 3 / 4 / 12 take the trick only
//!   when the profile's slot-tricks byte (+51) is set (`sub_82469EF0`; else the slot keeps the old trick);
//!   5 / 7 / 10 / 13 take it; 6 / 9 only validate; 8 is the manual value; 11 (revert) posts `Trick` and
//!   `SlideFs180` (fsrevert, id 4) / `SlideBs180` (bsrevert, id 5) (`sub_8246A1E8`).
//! - per-tick writer `sub_824696B8` (jump table 0x824698F8): categories 1-5, 7, 10, 12, 13 post the slot
//!   trick's `SkaterIntentTuple` list (`sub_82470ED0`: entry live from `start` to `end`, `end` < 0 = open);
//!   6 (handplant) posts `GrabWorld` 1.0; 8 `Manual`; 9 the powerslide keys; 11 nothing.
//! - takeoff board yaw (`sub_8246B1F8`) and its post in the air arm (`sub_8246B358` -> `sub_82471008`): see
//!   [`BoardYaw`].

use super::replay::{node_events, node_flags, ReplayLine};
use super::Vec3;
use crate::scoring::catalog;

/// Retail values (data-driven, mod-tunable through the game's settings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalSettings {
    /// Anticipation only this close to the line, metres (`0x82063B08`, 3.0).
    pub anticipation_distance: f32,
    /// Look-ahead in recorded 60 Hz frames (`sub_824691A0`, 60).
    pub anticipation_frames: u32,
    /// Posted only while the skater faces the line within this angle (`0x821A9C28`, pi/4).
    pub anticipation_heading: f32,
    /// Event nodes are skipped when the cursor crossed more nodes than this in one tick (3).
    pub max_crossed_nodes: u32,
    /// Mode 4 (skitch): the GrabWorld value posted every tick (`sub_8246FA30`, `0x8231A844`, 1.0; b65).
    pub skitch_grab: f32,
    /// The manual tunable of [`manual_value`] (`sub_82469338`: the tunable `742E...` looked up by hash, default
    /// `0x830D0850`, 0.5; also the board yaw's manual side): `Manual` = + it for a nosemanual, - it for a tailmanual.
    pub manual_value: f32,
}

impl Default for SignalSettings {
    fn default() -> Self {
        Self { anticipation_distance: 3.0, anticipation_frames: 60, anticipation_heading: core::f32::consts::FRAC_PI_4, max_crossed_nodes: 3, skitch_grab: 1.0, manual_value: 0.5 }
    }
}

/// One named signal for the ActionGraph intent map.
pub type Signal = (String, f32);

/// What the skater looks like to the signal writer this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalInput {
    /// The cursor's node and frames into its segment.
    pub node: u32,
    pub frame_in_segment: u32,
    /// The node the dispatcher handled last (`pc+820`); `None` right after spawn.
    pub last_node: Option<u32>,
    /// Distance of the skater from the line, metres (`pc+828`).
    pub off_line: f32,
    /// Angle between the skater's facing and the line, radians.
    pub heading_error: f32,
    /// A trick span is open (no new anticipation, no crouch).
    pub in_trick: bool,
}

/// The first start-trick node within `frames` recorded frames after (`node`, `into`)
/// (`sub_82456550`).
pub fn upcoming_start(line: &ReplayLine, node: u32, into: u32, frames: u32) -> Option<u32> {
    let mut budget = i64::from(frames) + i64::from(into);
    for i in (node as usize + 1)..line.nodes.len() {
        budget -= i64::from(line.nodes[i].frames);
        if budget < 0 {
            return None;
        }
        if line.nodes[i].event == node_events::START_TRICK {
            return Some(i as u32);
        }
    }
    None
}

/// The signals of this tick. `chosen` gives the trick started at a start-trick node (the npc_tricks
/// choice the replay cursor made; the recorded trick when it has none).
pub fn signals(line: &ReplayLine, s: &SignalSettings, input: &SignalInput, chosen: &dyn Fn(u32) -> i16, slot: &mut TrickSlot, slot_tricks: bool) -> Vec<Signal> {
    let mut out = Vec::new();
    // Anticipation.
    if input.off_line <= s.anticipation_distance && !input.in_trick && input.heading_error.abs() < s.anticipation_heading {
        if let Some(n) = upcoming_start(line, input.node, input.frame_in_segment, s.anticipation_frames) {
            let recorded = line.node_trick(&line.nodes[n as usize]);
            match catalog::category(recorded) {
                Some(1 | 2) => {
                    out.push(("AnticMag".into(), 1.0));
                    let angle = if super::npc_tricks::is_nollie(recorded) { core::f32::consts::PI } else { 0.0 };
                    out.push(("AnticAngle".into(), angle));
                }
                // `sub_824696B8` with `pc+788` = 8: the manual's side, `sub_82469338(pc+792)` (+ nose, - tail).
                Some(8) => out.push(("Manual".into(), manual_value(recorded, s.manual_value))),
                _ => {}
            }
        }
    }
    if !input.in_trick && line.nodes.get(input.node as usize).is_some_and(|n| n.flags & node_flags::CROUCHED != 0) {
        out.push(("Crouch".into(), 1.0));
    }
    // Dispatch: the newest start-trick node crossed this tick.
    let limit = if input.last_node.is_none() { 1 } else { s.max_crossed_nodes };
    let from = input.last_node.map_or(input.node, |l| l + 1);
    if input.node >= from && input.node + 1 - from <= limit {
        let newest = (from..=input.node).rev().find(|&n| line.nodes.get(n as usize).is_some_and(|x| x.event == node_events::START_TRICK || x.event == node_events::END_TRICK));
        if let Some(n) = newest {
            if line.nodes[n as usize].event == node_events::END_TRICK {
                slot.category = 0;
            } else {
                dispatch(line.node_trick(&line.nodes[n as usize]), chosen(n), slot, slot_tricks, s.manual_value, &mut out);
            }
        }
    }
    out
}

/// One timed intent of a trick (`Sk8::Score::SkaterIntentTuple`, 16 B: i32 intent index into
/// [`crate::input::skater_intent::SKATER_INTENTS`], f32 value, i32 first and last tick; last < 0 = until the slot
/// ends).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntentTuple {
    pub intent: u32,
    pub value: f32,
    pub start: i32,
    pub end: i32,
}

/// The PathController's trick slot: `pc+832` category (0 = none), `pc+844` trick, `pc+904` ticks since the start
/// node, `pc+836` the manual's value (set by a manual's dispatch). Plain data: serialisable state of the NPC.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrickSlot {
    pub category: u8,
    pub trick: i16,
    pub ticks: u32,
    pub manual: f32,
}

impl Default for TrickSlot {
    fn default() -> Self {
        Self { category: 0, trick: -1, ticks: 0, manual: 0.0 }
    }
}

fn valid(trick: i16) -> bool {
    usize::try_from(trick).is_ok_and(|i| i < catalog::IDENTIFIERS.len())
}

/// A start node: the category of the recorded trick goes in the slot, then the category's dispatch with the trick
/// the cursor chose (`sub_8246A2E0` from 0x8246A420).
fn dispatch(recorded: i16, trick: i16, slot: &mut TrickSlot, slot_tricks: bool, manual_tunable: f32, out: &mut Vec<Signal>) {
    slot.ticks = 0;
    slot.category = catalog::category(recorded).and_then(|c| u8::try_from(c).ok()).unwrap_or(0);
    let take = |slot: &mut TrickSlot| {
        let ok = valid(trick);
        if ok {
            slot.trick = trick;
        }
        ok
    };
    let ok = match slot.category {
        // The perform step (`sub_8246A080` -> `sub_82469448`, which also stores the trick). Powerslides (9) keep it
        // too until their writer arm is ported (it reads `pc+920`, not decoded).
        1 | 2 | 9 => {
            if let Some((name, ..)) = usize::try_from(trick).ok().and_then(|i| catalog::IDENTIFIERS.get(i)) {
                out.push(("Trick".into(), 1.0));
                out.push(((*name).to_string(), 1.0));
                out.push(("GestureSpeed".into(), 1.0));
                out.push(("TrickHeight".into(), 1.0));
                out.push(("DontMirrorTrick".into(), 1.0));
            }
            if slot.category != 9 && valid(trick) {
                slot.trick = trick;
            }
            valid(trick)
        }
        3 | 4 | 12 => !slot_tricks || take(slot),
        5 | 7 | 10 | 13 => take(slot),
        6 => valid(trick),
        // 0x8246A544: a manual also stores its side, `pc+836` = `sub_82469338(trick)`, for the writer.
        8 => {
            let ok = valid(trick);
            if ok {
                slot.manual = manual_value(trick, manual_tunable);
            }
            ok
        }
        11 => {
            let ok = valid(trick);
            if ok {
                out.push(("Trick".into(), 1.0));
                match trick {
                    4 => out.push(("SlideFs180".into(), 1.0)),
                    5 => out.push(("SlideBs180".into(), 1.0)),
                    _ => {}
                }
            }
            ok
        }
        _ => true,
    };
    if !ok {
        slot.category = 0;
    }
}

/// The per-tick writer's slot arm (`sub_824696B8` from 0x824698D0), then `pc+904 += 1`. `schedule` = the slot
/// trick's intent list; `handplant_grab` = the category-6 GrabWorld value (1.0, `0x8231A844`).
pub fn slot_signals(slot: &mut TrickSlot, schedule: &[IntentTuple], handplant_grab: f32) -> Vec<Signal> {
    let mut out = Vec::new();
    match slot.category {
        1..=5 | 7 | 10 | 12 | 13 if valid(slot.trick) => {
            let t = i64::from(slot.ticks);
            for e in schedule.iter().take(6) {
                let Some(name) = crate::input::skater_intent::name(e.intent) else { continue };
                if t < i64::from(e.start) || (e.end >= 0 && t > i64::from(e.end)) {
                    continue;
                }
                out.push((name.to_string(), e.value));
            }
        }
        6 => out.push(("GrabWorld".into(), handplant_grab)),
        // `loc_82469954`: a manual holds `Manual` = `pc+836` every tick until its end node.
        8 => out.push(("Manual".into(), slot.manual)),
        _ => {}
    }
    slot.ticks = slot.ticks.saturating_add(1);
    out
}

/// `sub_8296EC98(out, v, +Z, +Y)`: the angle from `v` to +Z (acos of the normalised dot), kept when
/// `cross(v, +Z) . +Y >= 0` (= -v.x >= 0), else `2 pi -` it; so `atan2(-v.x, v.z)` in [0, 2 pi). A zero vector gives 0
/// (`0x82165A10`), as does one with a squared length at or under 1e-4 (`0x8209BE90`).
pub fn retail_heading(v: Vec3) -> f32 {
    let n2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if n2 <= 1e-4 {
        return 0.0;
    }
    let angle = (v[2] / n2.sqrt()).clamp(-1.0, 1.0).acos();
    if -v[0] < 0.0 { core::f32::consts::TAU - angle } else { angle }
}

/// Retail values of the takeoff board yaw (data-driven defaults, mod-tunable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoardYawTuning {
    /// No tweak while the folded yaw is at or under this (rad, `0x822F9284`, 0.31416 = 18 deg).
    pub dead_zone: f32,
    /// `BoardAdjustMag` = |yaw| x this (`0x822F8C7C`, 6.0606).
    pub mag_scale: f32,
    /// `BoardAdjustAngle` with a manual ahead: nose-side and tail-side values (`0x822F94A8` / `0x822F94AC`).
    pub manual_ahead_angles: (f32, f32),
}

impl Default for BoardYawTuning {
    fn default() -> Self {
        Self { dead_zone: 0.31416, mag_scale: 6.0606, manual_ahead_angles: (-2.677, 0.257) }
    }
}

/// The takeoff board yaw: `pc+930` latched at this takeoff, `pc+931` outside the dead zone, `pc+856` angle,
/// `pc+860` magnitude. Plain data (serialisable NPC state).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoardYaw {
    pub latched: bool,
    pub set: bool,
    pub angle: f32,
    pub mag: f32,
}

impl BoardYaw {
    /// `sub_8246B1F8` on the takeoff edge, from the cursor node's recorded board and skater frames:
    /// d = heading(board forward) - heading(skater forward) ([`retail_heading`]), wrapped to [-pi, pi], folded to
    /// [-pi/2, pi/2] (nose and tail lie on the same board line); outside the dead zone the angle is +-pi/2 by the sign
    /// of d and the magnitude |d| x scale (not clamped here).
    pub fn takeoff(board: [f32; 4], skater: [f32; 4], t: &BoardYawTuning) -> Self {
        use core::f32::consts::{FRAC_PI_2, PI, TAU};
        let z = [0.0, 0.0, 1.0];
        let mut d = retail_heading(super::replay::rotate(board, z)) - retail_heading(super::replay::rotate(skater, z));
        if d > PI {
            d -= TAU;
        } else if d < -PI {
            d += TAU;
        }
        if d > FRAC_PI_2 {
            d -= PI;
        } else if d < -FRAC_PI_2 {
            d += PI;
        }
        let set = d.abs() > t.dead_zone;
        let angle = if !set { 0.0 } else if d >= 0.0 { FRAC_PI_2 } else { -FRAC_PI_2 };
        Self { latched: true, set, angle, mag: if set { d.abs() * t.mag_scale } else { 0.0 } }
    }

    /// The air arm's post (`sub_8246B358` -> `sub_82471008`, every airborne tick): with a manual anticipated ahead
    /// (`pc+796`, the slot not a manual, upcoming category 8) the manual's side, `manual` = [`manual_value`] of the
    /// upcoming trick; else the latched takeoff yaw; else nothing.
    pub fn post(&self, manual_ahead: Option<f32>, t: &BoardYawTuning) -> Option<[Signal; 2]> {
        let (angle, mag) = match manual_ahead {
            Some(v) => (if v > 0.0 { t.manual_ahead_angles.0 } else { t.manual_ahead_angles.1 }, v.abs()),
            None if self.latched && self.set => (self.angle, self.mag),
            None => return None,
        };
        Some([("BoardAdjustAngle".into(), angle), ("BoardAdjustMag".into(), mag)])
    }
}

/// `sub_82469338`: the manual value of a trick: + for nosemanual (2), - for tailmanual (3), else 0.
pub fn manual_value(trick: i16, tunable: f32) -> f32 {
    match trick {
        2 => tunable,
        3 => -tunable,
        _ => 0.0,
    }
}

/// The trick anticipated ahead (`pc+792`, with `pc+796` set, `sub_824691A0`): the recorded trick of the first start
/// node in the look-ahead, under the anticipation gates of [`signals`].
pub fn anticipated(line: &ReplayLine, s: &SignalSettings, input: &SignalInput) -> Option<i16> {
    if input.off_line > s.anticipation_distance || input.in_trick || input.heading_error.abs() >= s.anticipation_heading {
        return None;
    }
    let n = upcoming_start(line, input.node, input.frame_in_segment, s.anticipation_frames)?;
    Some(line.node_trick(&line.nodes[n as usize]))
}

/// Drop the trick dispatch's signals (the tail from `Trick`).
pub fn drop_trick_dispatch(out: &mut Vec<Signal>) {
    if let Some(i) = out.iter().position(|x| x.0 == "Trick") {
        out.truncate(i);
    }
}

/// Mode 4 of the obstacle avoider (`sub_8246FA30` at 0x8246FC78; b65): while the skater is not airborne the
/// controller posts GrabWorld every tick (the player's grab intent, so the riding skitch query and state 104 follow);
/// while it posts it or the skater is skitching (state 104) the trick dispatch is skipped (its signals dropped).
pub fn apply_skitch_mode(out: &mut Vec<Signal>, s: &SignalSettings, skitch_mode: bool, airborne: bool, skitching: bool) {
    let grab = skitch_mode && !airborne;
    if grab || skitching {
        drop_trick_dispatch(out);
    }
    if grab {
        out.push(("GrabWorld".into(), s.skitch_grab));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mode_4_grabs_and_skips_the_trick_dispatch() {
        let s = super::SignalSettings::default();
        let base: Vec<super::Signal> = vec![("Crouch".into(), 1.0), ("Trick".into(), 1.0), ("360flip".into(), 1.0), ("GestureSpeed".into(), 1.0)];
        let mut v = base.clone();
        super::apply_skitch_mode(&mut v, &s, true, false, false);
        assert_eq!(v, vec![("Crouch".into(), 1.0), ("GrabWorld".into(), 1.0)]);
        // Airborne: no grab, tricks kept.
        let mut v = base.clone();
        super::apply_skitch_mode(&mut v, &s, true, true, false);
        assert_eq!(v, base);
        // Skitching without mode 4: tricks dropped, no grab.
        let mut v = base.clone();
        super::apply_skitch_mode(&mut v, &s, false, false, true);
        assert_eq!(v, vec![("Crouch".into(), 1.0)]);
    }

    use super::*;
    use crate::living_world::replay::{ReplayJump, ReplayNode};

    /// Nodes 4 frames apart; a start-trick node at `at` carrying `trick`.
    fn line(at: usize, trick: i16, flags_at: &[(usize, u8)]) -> ReplayLine {
        ReplayLine {
            id: [0; 16],
            flags: 4,
            skill: 0,
            nodes: (0..40)
                .map(|i| ReplayNode {
                    position: [0.0, 0.0, i as f32],
                    step: [0.0; 3],
                    board: [128, 128, 128, 255],
                    skater: [128, 128, 128, 255],
                    frames: 4,
                    event: if i == at { node_events::START_TRICK } else { 0 },
                    flags: flags_at.iter().find(|f| f.0 == i).map_or(0, |f| f.1),
                    jump: (i == at).then_some(0),
                    width: [50, 50],
                })
                .collect(),
            jumps: vec![ReplayJump { start_position: [0.0; 3], start_velocity: [0.0; 3], offset: [0.0; 3], trick, spins: 0, flags: 0 }],
            groups: vec![],
        }
    }

    fn at(node: u32, last: Option<u32>) -> SignalInput {
        SignalInput { node, frame_in_segment: 0, last_node: last, off_line: 0.2, heading_error: 0.1, in_trick: false }
    }

    fn names(v: &[Signal]) -> Vec<&str> {
        v.iter().map(|s| s.0.as_str()).collect()
    }

    fn sig(l: &ReplayLine, s: &SignalSettings, input: &SignalInput, chosen: &dyn Fn(u32) -> i16) -> Vec<Signal> {
        signals(l, s, input, chosen, &mut TrickSlot::default(), true)
    }

    const BS_5_O: i16 = 6;
    const BS_NOSEBLUNT: i16 = 13;
    const FSREVERT: i16 = 4;
    const FS_HANDPLANT: i16 = 139;

    #[test]
    fn a_grind_only_fills_the_slot_and_its_schedule_runs_until_the_end_node() {
        let s = SignalSettings::default();
        let mut l = line(10, BS_5_O, &[]);
        l.nodes[14].event = node_events::END_TRICK;
        let mut slot = TrickSlot::default();
        let v = signals(&l, &s, &at(10, Some(9)), &|_| BS_5_O, &mut slot, true);
        assert!(!names(&v).contains(&"Trick"), "no perform signals for a grind: {v:?}");
        assert_eq!(slot, TrickSlot { category: 5, trick: BS_5_O, ticks: 0, manual: 0.0 });
        // The 5-0's schedule: PhysGrindUpDown -1 from the slot tick on, open-ended.
        let schedule = [IntentTuple { intent: 71, value: -1.0, start: 0, end: -1 }];
        for _ in 0..3 {
            assert_eq!(slot_signals(&mut slot, &schedule, 1.0), vec![("PhysGrindUpDown".to_string(), -1.0)]);
        }
        assert_eq!(slot.ticks, 3);
        signals(&l, &s, &at(14, Some(13)), &|_| BS_5_O, &mut slot, true);
        assert_eq!(slot.category, 0);
        assert!(slot_signals(&mut slot, &schedule, 1.0).is_empty());
    }

    #[test]
    fn schedule_windows_and_categories_follow_the_writer() {
        let schedule = [
            IntentTuple { intent: 17, value: 1.0, start: 0, end: 1 },
            IntentTuple { intent: 32, value: -1.0, start: 2, end: -1 },
            IntentTuple { intent: 0, value: 1.0, start: 0, end: -1 },
        ];
        let mut slot = TrickSlot { category: 10, trick: 20, ticks: 0, manual: 0.0 };
        let seen: Vec<Vec<Signal>> = (0..4).map(|_| slot_signals(&mut slot, &schedule, 1.0)).collect();
        assert_eq!(seen[0], vec![("LeftPush".to_string(), 1.0)]);
        assert_eq!(seen[1], vec![("LeftPush".to_string(), 1.0)]);
        assert_eq!(seen[2], vec![("TweakY".to_string(), -1.0)]);
        assert_eq!(seen[3], vec![("TweakY".to_string(), -1.0)]);
        // Handplant: GrabWorld every tick, no schedule. Manual: its side every tick (`pc+836`). Powerslide and revert:
        // nothing from the slot.
        let mut hp = TrickSlot { category: 6, trick: FS_HANDPLANT, ticks: 0, manual: 0.0 };
        assert_eq!(slot_signals(&mut hp, &schedule, 1.0), vec![("GrabWorld".to_string(), 1.0)]);
        let mut tail = TrickSlot { category: 8, trick: 3, ticks: 0, manual: -0.5 };
        assert_eq!(slot_signals(&mut tail, &schedule, 1.0), vec![("Manual".to_string(), -0.5)]);
        for c in [9, 11] {
            assert!(slot_signals(&mut TrickSlot { category: c, trick: 20, ticks: 0, manual: 0.0 }, &schedule, 1.0).is_empty());
        }
    }

    #[test]
    fn a_revert_posts_its_slide_key_and_the_profile_byte_gates_grabs() {
        let s = SignalSettings::default();
        let l = line(10, FSREVERT, &[]);
        let mut slot = TrickSlot::default();
        assert_eq!(signals(&l, &s, &SignalInput { heading_error: 1.0, ..at(10, Some(9)) }, &|_| FSREVERT, &mut slot, true), vec![("Trick".to_string(), 1.0), ("SlideFs180".to_string(), 1.0)]);
        assert_eq!(slot.category, 11);
        // A grab (category 4) without the profile byte keeps the old trick in the slot.
        let grab = (0..332).find(|&t| catalog::category(t) == Some(4)).unwrap();
        let l = line(10, grab, &[]);
        let mut slot = TrickSlot { category: 0, trick: BS_5_O, ticks: 7, manual: 0.0 };
        signals(&l, &s, &at(10, Some(9)), &|_| grab, &mut slot, false);
        assert_eq!(slot, TrickSlot { category: 4, trick: BS_5_O, ticks: 0, manual: 0.0 });
        signals(&l, &s, &at(10, Some(9)), &|_| grab, &mut slot, true);
        assert_eq!(slot.trick, grab);
    }

    fn yaw(deg: f32) -> [u8; 4] {
        let h = deg.to_radians() / 2.0;
        [128, (128.0 + 127.0 * h.sin()).round() as u8, 128, (128.0 + 127.0 * h.cos()).round() as u8]
    }

    #[test]
    fn the_retail_heading_is_atan2_minus_x_z_in_zero_to_two_pi() {
        for deg in [0.0f32, 30.0, 90.0, 179.0, 181.0, 270.0, 359.0] {
            let r = deg.to_radians();
            // A forward rotated by +deg about +Y is (sin, 0, cos); its retail heading is 2 pi - deg (mod 2 pi).
            let h = retail_heading([r.sin(), 0.0, r.cos()]);
            let want = (core::f32::consts::TAU - r).rem_euclid(core::f32::consts::TAU);
            let diff = (h - want).abs();
            assert!(diff < 1e-3 || (diff - core::f32::consts::TAU).abs() < 1e-3, "{deg}: {h} vs {want}");
        }
        assert_eq!(retail_heading([0.0; 3]), 0.0);
    }

    #[test]
    fn the_takeoff_yaw_folds_nose_and_tail_and_has_a_dead_zone() {
        use core::f32::consts::FRAC_PI_2;
        let t = BoardYawTuning::default();
        // Board turned +80 deg about +Y from the skater (a noseblunt-like pose): d = heading(board) - heading(skater)
        // = -80 deg, so the angle is -pi/2 and the magnitude 80 deg x 6.0606. (Exactly 90 deg is the fold edge: the
        // byte rounding of the recorded frames decides the side there, as in retail.)
        let b = BoardYaw::takeoff(decode(yaw(80.0)), decode(yaw(0.0)), &t);
        assert!(b.set && b.latched);
        assert!((b.angle + FRAC_PI_2).abs() < 1e-6, "{b:?}");
        assert!((b.mag - 80f32.to_radians() * 6.0606).abs() < 0.05, "{b:?}");
        // The other side and the reversed board (tail first) give the mirrored / same results.
        assert!((BoardYaw::takeoff(decode(yaw(-80.0)), decode(yaw(0.0)), &t).angle - FRAC_PI_2).abs() < 1e-6);
        let tail = BoardYaw::takeoff(decode(yaw(-160.0)), decode(yaw(0.0)), &t);
        assert!(tail.set && tail.angle < 0.0, "{tail:?}");
        // Board along the skater (50-50) or reversed (180): no tweak; 10 deg is inside the 18 deg dead zone.
        for deg in [0.0, 180.0, 10.0, -10.0, 170.0] {
            let b = BoardYaw::takeoff(decode(yaw(deg)), decode(yaw(0.0)), &t);
            assert!(!b.set && b.post(None, &t).is_none(), "{deg}: {b:?}");
        }
        // Posting: a manual ahead wins; a nosemanual sends the nose-side angle.
        let p = b.post(Some(manual_value(2, SignalSettings::default().manual_value)), &t).unwrap();
        assert_eq!(p, [("BoardAdjustAngle".to_string(), -2.677), ("BoardAdjustMag".to_string(), 0.5)]);
        assert_eq!(BoardYaw::takeoff(decode(yaw(80.0)), decode(yaw(0.0)), &t).post(None, &t).unwrap()[0], ("BoardAdjustAngle".to_string(), -FRAC_PI_2));
    }

    fn decode(raw: [u8; 4]) -> [f32; 4] {
        crate::living_world::replay::decode_orientation(raw)
    }

    #[test]
    fn a_grind_on_a_noseblunt_node_posts_nothing_at_dispatch() {
        let s = SignalSettings::default();
        let l = line(10, BS_NOSEBLUNT, &[]);
        let mut slot = TrickSlot::default();
        let v = signals(&l, &s, &SignalInput { heading_error: 1.0, ..at(10, Some(9)) }, &|_| BS_NOSEBLUNT, &mut slot, true);
        assert!(v.is_empty(), "{v:?}");
        assert_eq!((slot.category, slot.trick), (5, BS_NOSEBLUNT));
    }

    #[test]
    fn an_ollie_within_a_second_is_anticipated_then_started() {
        // 360flip (id 85, category 2) at node 20: 80 frames from node 0, 60 from node 5.
        let l = line(20, 85, &[]);
        let s = SignalSettings::default();
        assert!(sig(&l, &s, &at(0, Some(0)), &|_| 85).is_empty());
        let near = sig(&l, &s, &at(5, Some(4)), &|_| 85);
        assert_eq!(names(&near), ["AnticMag", "AnticAngle"]);
        assert_eq!(near[1].1, 0.0);
        // Too far off the line or facing away: nothing.
        assert!(sig(&l, &s, &SignalInput { off_line: 3.5, ..at(5, Some(4)) }, &|_| 85).is_empty());
        assert!(sig(&l, &s, &SignalInput { heading_error: 1.0, ..at(5, Some(4)) }, &|_| 85).is_empty());
        // Reaching the node: the trick, by its scorable name.
        let go = sig(&l, &s, &at(20, Some(19)), &|_| 85);
        assert_eq!(names(&go), ["Trick", "360flip", "GestureSpeed", "TrickHeight", "DontMirrorTrick"]);
    }

    #[test]
    fn the_chosen_trick_is_started_and_a_nollie_is_anticipated_backwards() {
        let nollie = catalog::IDENTIFIERS.iter().position(|t| t.0.starts_with('n') && t.2 == 1).unwrap() as i16;
        let l = line(10, nollie, &[]);
        let s = SignalSettings::default();
        let v = sig(&l, &s, &at(5, Some(4)), &|_| nollie);
        assert_eq!(v[1], ("AnticAngle".to_string(), core::f32::consts::PI));
        let go = sig(&l, &s, &at(10, Some(9)), &|_| 85);
        assert_eq!(go[1].0, "360flip");
    }

    /// A tailmanual (3) is anticipated with its side (`Manual` = -0.5, not a nose-side 1.0) and held with that value
    /// every tick from its start node until its end node (`pc+836`, `loc_82469954`); before, the manual dropped at its
    /// start node and the ollie right after it never popped (andrew_reynolds at DownTown node 58).
    #[test]
    fn a_manual_is_anticipated_and_held_with_its_side() {
        let mut l = line(10, 3, &[]);
        l.nodes[14].event = node_events::END_TRICK;
        let s = SignalSettings::default();
        let ahead = sig(&l, &s, &at(5, Some(4)), &|_| 3);
        assert_eq!(ahead, vec![("Manual".to_string(), -0.5)]);
        let mut slot = TrickSlot::default();
        let start = signals(&l, &s, &at(10, Some(9)), &|_| 3, &mut slot, true);
        assert!(names(&start).iter().all(|n| *n != "Trick"), "a manual posts no Trick: {start:?}");
        assert_eq!((slot.category, slot.manual), (8, -0.5));
        for _ in 0..3 {
            assert_eq!(slot_signals(&mut slot, &[], 1.0), vec![("Manual".to_string(), -0.5)]);
        }
        signals(&l, &s, &SignalInput { in_trick: true, ..at(14, Some(13)) }, &|_| 3, &mut slot, true);
        assert_eq!(slot.category, 0);
        assert!(slot_signals(&mut slot, &[], 1.0).is_empty(), "nothing after the end node");
        assert_eq!(manual_value(2, s.manual_value), 0.5, "a nosemanual's side is +");
    }

    #[test]
    fn crossing_too_many_nodes_skips_the_event_and_crouch_follows_the_node() {
        let l = line(10, 85, &[(3, node_flags::CROUCHED)]);
        let s = SignalSettings::default();
        assert!(names(&sig(&l, &s, &at(10, Some(5)), &|_| 85)).iter().all(|n| *n != "Trick"));
        assert!(names(&sig(&l, &s, &at(10, Some(7)), &|_| 85)).contains(&"Trick"));
        assert!(names(&sig(&l, &s, &at(3, Some(2)), &|_| 85)).contains(&"Crouch"));
        assert!(!names(&sig(&l, &s, &SignalInput { in_trick: true, ..at(3, Some(2)) }, &|_| 85)).contains(&"Crouch"));
    }
}
