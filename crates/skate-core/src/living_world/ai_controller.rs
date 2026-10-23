//! NPC skater controller sub-modes for the obstacle avoider's modes 6 and 7 (8246D560, 8246F938, 8246FE38).
//! Retail mechanism: docs/hails-additions/26e-living-world-skater-ai.md#avoider-modes-6-and-7

/// Retail numbers; every field is data a mod can override.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerSettings {
    /// Nodes ahead of the cursor for the sub-mode 3 reposition (5).
    pub reposition_ahead: u32,
    /// Nodes past the chosen node (2).
    pub reposition_past: u32,
    /// A remembered prop node wins when it lies 1..this nodes past the choice (20, exclusive).
    pub remembered_window: u32,
    /// Clock units between two repositions (60; the clock unit is not decoded, b66).
    pub reposition_interval: u64,
    /// The `WipeOutRequest` value (1.0, `0x8231A844`).
    pub wipe_out: f32,
    /// Safe node picker (82455728): jump / flag frame gaps, slope, trick class, a run of good nodes: docs/hails-additions/26e-living-world-skater-ai.md#safe-node-picker
    pub safe_jump_frames: u32,
    pub safe_flag_frames: u32,
    pub safe_slope: f32,
    pub safe_run: u32,
    /// The stuck tracker (`sub_82468240`, b111): no-progress tick limits for stages 1 to 6 (stage 7 and up: 0).
    pub stuck_limits: [u32; 6],
    /// A stall clears after more than `stuck_clear_ticks` (25) progress ticks once the skater is more than
    /// `stuck_clear_distance` (5 m, `0x8209994C` 25.0 squared, horizontal) from where it stalled.
    pub stuck_clear_ticks: u32,
    pub stuck_clear_distance: f32,
    /// Stage 4: total ticks (no progress + progress) before stage 5 (180); the clear in stage 4 needs as many.
    pub stuck_hold_ticks: u32,
    /// The tracker counts only after more than `stuck_gate_ticks` (120) ticks since the last full reset (`pc+908`).
    pub stuck_gate_ticks: u32,
    /// At the line start (A at node 0, fraction <= `stuck_start_fraction`, `0x82063A48` 0.001) a tick only counts
    /// as progress when the distance to A shrank by more than `stuck_start_approach` (`0x8208EA80` 0.03 m).
    pub stuck_start_fraction: f32,
    pub stuck_start_approach: f32,
    /// The stage 2 re-snap's skip-ahead budget in summed node frames (`sub_8246D168`, 240).
    pub resnap_frames: u32,
}

impl Default for ControllerSettings {
    fn default() -> Self {
        Self {
            reposition_ahead: 5, reposition_past: 2, remembered_window: 20, reposition_interval: 60, wipe_out: 1.0, safe_jump_frames: 45, safe_flag_frames: 15, safe_slope: 1.0, safe_run: 3,
            stuck_limits: [30, 30, 2, 180, 30, 3], stuck_clear_ticks: 25, stuck_clear_distance: 5.0, stuck_hold_ticks: 180, stuck_gate_ticks: 120, stuck_start_fraction: 0.001, stuck_start_approach: 0.03,
            resnap_frames: 240,
        }
    }
}

/// `ctrl+568` values used here: the stuck tracker's stage (`sub_82468240`, b111), shared with the avoider's step off
/// and low prop handling (b66 / b70).
pub mod sub_mode {
    pub const NORMAL: u8 = 0;
    /// A stall began (anchor recorded).
    pub const STALLED: u8 = 1;
    /// Re-snap the line cursor once (`sub_8246D168`).
    pub const RESNAP: u8 = 2;
    /// Walk back to the line (the reposition request); also the avoider's step off.
    pub const STEP_OFF: u8 = 3;
    /// Hold: no node actions, no speed shaping; also the avoider's low prop.
    pub const LOW_PROP: u8 = 4;
    /// Bail (`WipeOutRequest` every tick); also the avoider's second step-off tick.
    pub const STEP_OFF_BAIL: u8 = 5;
    /// Respawn at a safe node (`sub_8246ED68` / `sub_8246E558`).
    pub const RESPAWN: u8 = 6;
}

/// The stuck tracker's block besides the stage (`pc+528..+583` minus `+568`; b111 section 2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StuckTracker {
    /// `+528`: where the stall began; `+560/+564`: cursor A then.
    pub anchor: super::Vec3,
    pub anchor_cursor: (u32, f32),
    /// `+572` / `+576`: no-progress and progress ticks in this stage.
    pub no_progress: u32,
    pub progress: u32,
    /// `+580`: |skater - A| this tick.
    pub dist: f32,
}

/// The controller fields this logic owns (`+568`, `+941`, `+943` / `+756`, `+912`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControllerState {
    pub sub_mode: u8,
    pub reposition_request: bool,
    pub remembered_node: Option<u32>,
    pub last_reposition: Option<u64>,
    pub stuck: StuckTracker,
    /// `pc+908`: ticks since the last full reset (the tracker's gate).
    pub gate_ticks: u32,
}

/// `sub_8246D560` for mode 6: enter sub-mode 4 (its exit is not decoded yet, b66).
pub fn enter_low_prop(state: &mut ControllerState) {
    if state.sub_mode < sub_mode::LOW_PROP {
        state.sub_mode = sub_mode::LOW_PROP;
    }
}

/// `sub_8246F938` for one tick; returns the `WipeOutRequest` value to post, if any. `online` is the flag at
/// `0x830CFE5C`+608 (meaning open; offline 0 runs the reposition step first).
pub fn step_off(state: &mut ControllerState, s: &ControllerSettings, mode_7: bool, airborne: bool, online: bool, prop_node: u32) -> Option<f32> {
    if !mode_7 || airborne {
        return None;
    }
    if !online && state.sub_mode < sub_mode::STEP_OFF {
        state.sub_mode = sub_mode::STEP_OFF;
        state.reposition_request = true;
        return None;
    }
    if state.sub_mode < sub_mode::STEP_OFF_BAIL {
        state.sub_mode = sub_mode::STEP_OFF_BAIL;
        state.remembered_node = Some(prop_node);
        return Some(s.wipe_out);
    }
    None
}

/// `sub_8246FE38`'s node choice.
pub fn reposition_node(state: &ControllerState, s: &ControllerSettings, mode_7: bool, cursor: u32, prop_node: u32, node_count: u32) -> u32 {
    let mut node = cursor;
    if state.sub_mode == sub_mode::STEP_OFF {
        let mut d = s.reposition_ahead;
        if mode_7 && prop_node > cursor && prop_node - cursor > d {
            d = prop_node - cursor;
        }
        node = cursor + d;
    }
    node += s.reposition_past;
    if let Some(r) = state.remembered_node {
        let r = r + s.reposition_past;
        if r > node && r - node < s.remembered_window {
            node = r;
        }
    }
    node.min(node_count.saturating_sub(1))
}

/// Off-board toggle the line controller presses (b117, 8246FA30 -> 824712E0): docs/hails-additions/26e-living-world-skater-ai.md#off-board-toggle
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OffBoardToggle {
    pub mode: u8,
    pub ticks: u32,
}

impl OffBoardToggle {
    /// `824712E0`: mode 1 presses while on the board, mode 2 while off it; never while toggling (`byte_160`) or with a
    /// press pending (`pc+946`); the count is tested, reset on a press, then always counted up. True = send the
    /// `NewToggleOffBoardState` / `ToggleOffBoardState` pair.
    pub fn press(&mut self, period: u32, off_board: bool, toggling: bool) -> bool {
        let press = self.mode != 0 && self.ticks > period && !toggling && ((self.mode == 1 && !off_board) || (self.mode == 2 && off_board));
        if press {
            self.ticks = 0;
        }
        self.ticks += 1;
        press
    }

    /// The end of A's path tick: the mode for the next tick.
    pub fn line_mode(&mut self, stay_off: bool) {
        self.mode = if stay_off { 1 } else { 2 };
    }
}

/// 8246ED68: node a bailed AI skater is placed at again (picked at bail start, 8246EE30): docs/hails-additions/26e-living-world-skater-ai.md#bail-respawn-node
pub fn respawn_node(state: &ControllerState, s: &ControllerSettings, search_start: u32, line: &super::replay::ReplayLine) -> u32 {
    let mut node = search_start + s.reposition_past;
    if let Some(r) = state.remembered_node {
        let r = r + s.reposition_past;
        if r > node && r - node < s.remembered_window {
            node = r;
        }
    }
    safe_node(line, node.min((line.nodes.len() as u32).saturating_sub(1)), s)
}

/// `sub_82455348`: the node's jump is a ground trick (category 5 grind / slide, 8 manual, 9 powerslide, or 4 grab
/// for the ground grabs coffin 60 and gnd_bsgrab / gnd_dblgrab / gnd_fsgrab 67 to 69; `sub_824545F0` reads the
/// category, table `0x820862A8` +16).
fn ground_trick(line: &super::replay::ReplayLine, node: &super::replay::ReplayNode) -> bool {
    let Some(trick) = node.jump.and_then(|j| line.jumps.get(j as usize)).map(|j| j.trick) else { return false };
    match crate::scoring::catalog::category(trick) {
        Some(5 | 8 | 9) => true,
        Some(4) => matches!(trick, 60 | 67 | 68 | 69),
        _ => false,
    }
}

/// `sub_82455728(line, start)`: the safe node to re-attach at, from `start` on (the counters are primed over the
/// nodes before it); the last node when none qualifies.
pub fn safe_node(line: &super::replay::ReplayLine, start: u32, s: &ControllerSettings) -> u32 {
    use super::replay::node_events;
    const RESET: u32 = 7200;
    let (mut since_jump, mut since_flag, mut class) = (RESET, RESET, 0usize);
    let mut count = |n: &super::replay::ReplayNode| {
        let jump = n.jump.and_then(|j| line.jumps.get(j as usize));
        // Jump marker (ext +39 bit 0), a ground trick's start, else count the frames.
        since_jump = if jump.is_some_and(|j| j.flags & 1 != 0) {
            0
        } else if n.event == node_events::START_TRICK && ground_trick(line, n) {
            RESET
        } else {
            since_jump + u32::from(n.frames)
        };
        since_flag = if n.flags & 0x4 != 0 { 0 } else { since_flag + u32::from(n.frames) };
        if n.event == node_events::START_TRICK {
            class = jump.and_then(|j| crate::scoring::catalog::category(j.trick)).unwrap_or(0);
        } else if n.event == node_events::END_TRICK {
            class = 0;
        }
        (since_jump, since_flag, class)
    };
    let nodes = &line.nodes;
    for n in nodes.iter().take(start as usize) {
        count(n);
    }
    let mut run = 0;
    let mut first = 0u32;
    for i in start as usize..nodes.len() {
        let n = &nodes[i];
        let (a, b, c) = count(n);
        let h2 = n.step[0] * n.step[0] + n.step[2] * n.step[2];
        let slope = if h2 > 0.001 { n.step[1] * n.step[1] / h2 } else { 0.0 };
        let good = a > s.safe_jump_frames && b > s.safe_flag_frames && slope < s.safe_slope && matches!(c, 0 | 4 | 8);
        if !good {
            run = 0;
            continue;
        }
        if run == 0 {
            first = i as u32;
        }
        run += 1;
        if run >= s.safe_run {
            return first;
        }
    }
    (nodes.len() as u32).saturating_sub(1)
}

/// `sub_8246FE38`: the node to move to when the one-shot request may run now (offline, throttled); the request is
/// cleared either way. The caller reports success with [`reposition_done`]. The chosen node then goes through
/// [`safe_node`] before the swap to controller B.
pub fn take_reposition(state: &mut ControllerState, s: &ControllerSettings, online: bool, now: u64, mode_7: bool, cursor: u32, prop_node: u32, node_count: u32) -> Option<u32> {
    let pending = std::mem::take(&mut state.reposition_request);
    if !pending || online || state.last_reposition.is_some_and(|last| now < last + s.reposition_interval) {
        return None;
    }
    let node = reposition_node(state, s, mode_7, cursor, prop_node, node_count);
    state.last_reposition = Some(now);
    Some(node)
}

/// A successful move resets the controller (sub-mode 0 and the stuck tracker; the throttle time and the tracker's
/// gate count `pc+908` are kept: `sub_8246FE38` zeroes the tracker block only).
pub fn reposition_done(state: &mut ControllerState) {
    *state = ControllerState { last_reposition: state.last_reposition, gate_ticks: state.gate_ticks, ..Default::default() };
}

/// The path state reset on a line switch (`sub_82468AC8`, b111 section 5 / b118): it clears the tracker block
/// `pc+528..+583` (stage included; the stage 2 re-snap saves and restores it around this reset). The gate count
/// `pc+908` and the throttle stay.
pub fn path_reset(state: &mut ControllerState) {
    state.stuck = StuckTracker::default();
    state.sub_mode = sub_mode::NORMAL;
}

/// The full reset after the skater is placed again (`sub_82468C10`, from `sub_8246E558`): sub-mode, tracker and its
/// gate count (`pc+908` = 0, so the tracker waits 120 ticks again); the throttle time is kept.
pub fn full_reset(state: &mut ControllerState) {
    *state = ControllerState { last_reposition: state.last_reposition, ..Default::default() };
}

/// 82468240 for one tick (b111 section 4): stuck tracking stages: docs/hails-additions/26e-living-world-skater-ai.md#track-stuck
pub fn track_stuck(state: &mut ControllerState, s: &ControllerSettings, pos: super::Vec3, a: (u32, f32), a_pos: super::Vec3, progress: bool, can_track: bool) {
    state.gate_ticks = state.gate_ticks.saturating_add(1);
    let gate = state.gate_ticks > s.stuck_gate_ticks;
    let t = &mut state.stuck;
    let old = t.dist;
    t.dist = ((pos[0] - a_pos[0]).powi(2) + (pos[1] - a_pos[1]).powi(2) + (pos[2] - a_pos[2]).powi(2)).sqrt();
    let mut progress = progress;
    if progress {
        let past_start = a.0 > 0 || a.1 > s.stuck_start_fraction;
        if !past_start && old > 0.0 && t.dist > old - s.stuck_start_approach {
            progress = false;
        }
    }
    if progress {
        if state.sub_mode > sub_mode::NORMAL {
            t.progress += 1;
            let away = (pos[0] - t.anchor[0]).powi(2) + (pos[2] - t.anchor[2]).powi(2);
            if t.progress > s.stuck_clear_ticks
                && away > s.stuck_clear_distance * s.stuck_clear_distance
                && (state.sub_mode != sub_mode::LOW_PROP || t.no_progress + t.progress >= s.stuck_hold_ticks)
            {
                *t = StuckTracker { dist: t.dist, ..Default::default() };
                state.sub_mode = sub_mode::NORMAL;
            }
        }
    } else if can_track && gate {
        if state.sub_mode == sub_mode::NORMAL {
            *t = StuckTracker { anchor: pos, anchor_cursor: a, no_progress: 0, progress: 0, dist: t.dist };
            state.sub_mode = sub_mode::STALLED;
        }
        t.no_progress += 1;
        let limit = usize::from(state.sub_mode).checked_sub(1).and_then(|i| s.stuck_limits.get(i)).copied().unwrap_or(0);
        if t.no_progress > limit {
            t.no_progress = 0;
            t.progress = 0;
            state.sub_mode = state.sub_mode.saturating_add(1);
        }
    }
    if state.sub_mode == sub_mode::LOW_PROP && state.stuck.progress + state.stuck.no_progress > s.stuck_hold_ticks {
        state.stuck.progress = 0;
        state.stuck.no_progress = 0;
        state.sub_mode = sub_mode::STEP_OFF_BAIL;
    }
}

/// What the stage asks for this tick (`sub_8246F818`, run before the projection; b111 section 5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StuckAction {
    None,
    /// Stage 2's first tick: re-snap the cursor ([`resnap_node`]) and reset the path state, keeping the tracker.
    Resnap,
    /// Stage 5: post `WipeOutRequest` with this value.
    WipeOut(f32),
    /// Stage 6: place the skater at [`respawn_node`] and fully reset.
    Respawn,
}

/// `sub_8246F818`: stage 3 raises the reposition request (offline only: `online` = `0x830CFE5C`+608), which
/// [`take_reposition`] turns into the walk back (5+ nodes ahead); the other stages return their action.
pub fn stuck_action(state: &mut ControllerState, s: &ControllerSettings, online: bool) -> StuckAction {
    match state.sub_mode {
        sub_mode::RESNAP if state.stuck.no_progress == 0 && state.stuck.progress == 0 => StuckAction::Resnap,
        sub_mode::STEP_OFF => {
            if !online {
                state.reposition_request = true;
            }
            StuckAction::None
        }
        sub_mode::STEP_OFF_BAIL => StuckAction::WipeOut(s.wipe_out),
        sub_mode::RESPAWN => StuckAction::Respawn,
        _ => StuckAction::None,
    }
}

/// `sub_8246D168`'s node: when the line climbs steeper than `safe_slope` (1.0) from `search` to `search + 2`, the
/// first node after `search + 2` lower than `search`, within `resnap_frames` (240) summed node frames (node byte `+36` = our `frames`,
/// `skate-data` `aipath.rs`: 4 s of 60 Hz recording); otherwise (or none found) the node nearest `pos` over the line.
pub fn resnap_node(line: &super::replay::ReplayLine, s: &ControllerSettings, search: u32, pos: super::Vec3) -> u32 {
    let budget = s.resnap_frames;
    let nodes = &line.nodes;
    let i = search as usize;
    if i + 2 < nodes.len() {
        let (a, b) = (nodes[i].position, nodes[i + 2].position);
        let h = ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        let slope = if h > 0.001 { (b[1] - a[1]) / h } else { 0.0 };
        if slope > s.safe_slope {
            let mut sum = 0u32;
            // `8246D168` tests the lower node before the budget, and both only past `search + 2` (b524).
            for (j, n) in nodes.iter().enumerate().skip(i + 1) {
                sum += u32::from(n.frames);
                if j > i + 2 {
                    if n.position[1] < a[1] {
                        return j as u32;
                    }
                    if sum > budget {
                        break;
                    }
                }
            }
        }
    }
    super::path_follow::nearest_node(line, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// b524 (`8246D168`): on a steep start the scan tests "lower than the search node" before the 240-frame budget,
    /// and both only past `search + 2`, so a lower node after a long first segment is still found.
    #[test]
    fn the_steep_resnap_finds_a_lower_node_after_a_long_segment() {
        use crate::living_world::replay::{ReplayLine, ReplayNode};
        let node = |y: f32, z: f32, frames: u8| ReplayNode { position: [0.0, y, z], step: [0.0, 0.0, 0.1], board: [128; 4], skater: [128; 4], frames, event: 0, flags: 0, jump: None, width: [25, 25] };
        // Up 10 m over 2 m (slope 5), node 1 alone carries 250 frames, node 3 is below the start.
        let nodes = vec![node(0.0, 0.0, 0), node(5.0, 1.0, 250), node(10.0, 2.0, 10), node(-1.0, 3.0, 10), node(-2.0, 4.0, 10)];
        let line = ReplayLine { id: [0; 16], flags: 0, skill: -1, nodes, jumps: Vec::new(), groups: Vec::new() };
        let s = ControllerSettings::default();
        assert_eq!(resnap_node(&line, &s, 0, [0.0, 10.0, 2.0]), 3);
    }

    /// A skater that stops on its line and never moves again: the gate waits 120 ticks, then the stages climb at
    /// 30 / 30 / 2 / 180 / 30 no-progress ticks past their limit (b111 counts one tick later: re-snap ~32, walk back ~63, bail ~247,
    /// respawn ~278 ticks into the stall), and each stage asks for its action.
    #[test]
    fn a_skater_that_never_moves_climbs_every_stage() {
        let s = ControllerSettings::default();
        let mut c = ControllerState::default();
        let (pos, a, a_pos) = ([0.0, 0.0, 0.0], (10, 0.5), [0.0, 0.0, 1.0]);
        for _ in 0..120 {
            track_stuck(&mut c, &s, pos, a, a_pos, false, true);
        }
        assert_eq!(c.sub_mode, sub_mode::NORMAL, "the gate holds the tracker off for 120 ticks");
        let mut entered = Vec::new();
        for tick in 1..=400u32 {
            let before = c.sub_mode;
            track_stuck(&mut c, &s, pos, a, a_pos, false, true);
            if c.sub_mode != before {
                entered.push((c.sub_mode, tick));
            }
            if c.sub_mode == sub_mode::RESPAWN {
                break;
            }
        }
        assert_eq!(entered, vec![(1, 1), (2, 31), (3, 62), (4, 65), (5, 246), (6, 277)]);
        assert_eq!(stuck_action(&mut c, &s, false), StuckAction::Respawn);
        c.sub_mode = sub_mode::RESNAP;
        c.stuck.no_progress = 0;
        c.stuck.progress = 0;
        assert_eq!(stuck_action(&mut c, &s, false), StuckAction::Resnap);
        c.stuck.no_progress = 1;
        assert_eq!(stuck_action(&mut c, &s, false), StuckAction::None, "the re-snap runs on the stage's first tick only");
        c.sub_mode = sub_mode::STEP_OFF;
        assert_eq!(stuck_action(&mut c, &s, false), StuckAction::None);
        assert!(c.reposition_request, "stage 3 raises the walk-back request offline");
        c.reposition_request = false;
        assert_eq!(stuck_action(&mut c, &s, true), StuckAction::None);
        assert!(!c.reposition_request, "not online");
        c.sub_mode = sub_mode::STEP_OFF_BAIL;
        assert_eq!(stuck_action(&mut c, &s, false), StuckAction::WipeOut(1.0));
    }

    /// Progress ticks never lower the stage; the stall clears only after more than 25 of them once the skater is
    /// more than 5 m (horizontally) from where it stalled.
    #[test]
    fn a_stall_clears_after_25_progress_ticks_more_than_5_m_away() {
        let s = ControllerSettings::default();
        let mut c = ControllerState { gate_ticks: 1000, ..Default::default() };
        let a = (10, 0.5);
        for _ in 0..5 {
            track_stuck(&mut c, &s, [0.0, 0.0, 0.0], a, [0.0, 0.0, 1.0], false, true);
        }
        assert_eq!(c.sub_mode, sub_mode::STALLED);
        // Inching along within 5 m (and 10 m up: the test is horizontal): stays stalled.
        for _ in 0..60 {
            track_stuck(&mut c, &s, [4.0, 10.0, 0.0], a, [4.0, 10.0, 1.0], true, true);
        }
        assert_eq!(c.sub_mode, sub_mode::STALLED);
        for _ in 0..26 {
            track_stuck(&mut c, &s, [6.0, 0.0, 0.0], a, [6.0, 0.0, 1.0], true, true);
        }
        assert_eq!(c.sub_mode, sub_mode::NORMAL);
        assert_eq!(c.stuck.no_progress + c.stuck.progress, 0);
    }

    /// While the tracker may not count (airborne, scripted, toggling) nothing starts; at the line start a tick is only
    /// progress when the skater closed in on cursor A by more than 3 cm.
    #[test]
    fn untracked_ticks_and_the_line_start_rule() {
        let s = ControllerSettings::default();
        let mut c = ControllerState { gate_ticks: 1000, ..Default::default() };
        for _ in 0..100 {
            track_stuck(&mut c, &s, [0.0; 3], (10, 0.5), [0.0, 0.0, 1.0], false, false);
        }
        assert_eq!(c.sub_mode, sub_mode::NORMAL);
        // At node 0: "progress" while standing 1 m from A is no progress.
        track_stuck(&mut c, &s, [0.0; 3], (0, 0.0), [0.0, 0.0, 1.0], true, true);
        track_stuck(&mut c, &s, [0.0; 3], (0, 0.0), [0.0, 0.0, 1.0], true, true);
        assert_eq!(c.sub_mode, sub_mode::STALLED);
        // Closing in by 5 cm a tick counts.
        let before = c.stuck.progress;
        track_stuck(&mut c, &s, [0.0, 0.0, 0.05], (0, 0.0), [0.0, 0.0, 1.0], true, true);
        assert_eq!(c.stuck.progress, before + 1);
    }

    /// Stage 4 (hold, also the avoider's low prop) moves on to the bail after 180 ticks in it, progress or not.
    #[test]
    fn stage_4_moves_on_to_the_bail_after_180_ticks() {
        let s = ControllerSettings::default();
        let mut c = ControllerState { gate_ticks: 1000, sub_mode: sub_mode::LOW_PROP, ..Default::default() };
        for _ in 0..181 {
            track_stuck(&mut c, &s, [0.0; 3], (10, 0.5), [0.0, 0.0, 1.0], true, true);
        }
        assert_eq!(c.sub_mode, sub_mode::STEP_OFF_BAIL);
    }

    /// The walk back clears the tracker but keeps its gate count; a placement resets the gate too.
    #[test]
    fn resets_keep_or_clear_the_gate() {
        let mut c = ControllerState { gate_ticks: 500, sub_mode: sub_mode::STEP_OFF, last_reposition: Some(7), ..Default::default() };
        reposition_done(&mut c);
        assert_eq!((c.sub_mode, c.gate_ticks, c.last_reposition), (0, 500, Some(7)));
        c.sub_mode = sub_mode::STEP_OFF_BAIL;
        full_reset(&mut c);
        assert_eq!((c.sub_mode, c.gate_ticks, c.last_reposition), (0, 0, Some(7)));
    }

    #[test]
    fn mode_7_requests_a_reposition_then_wipes_out_once() {
        let s = ControllerSettings::default();
        let mut c = ControllerState::default();
        // Airborne: nothing.
        assert_eq!(step_off(&mut c, &s, true, true, false, 40), None);
        assert_eq!(c, ControllerState::default());
        assert_eq!(step_off(&mut c, &s, true, false, false, 40), None);
        assert!(c.reposition_request && c.sub_mode == sub_mode::STEP_OFF);
        assert_eq!(step_off(&mut c, &s, true, false, false, 40), Some(1.0));
        assert_eq!((c.sub_mode, c.remembered_node), (sub_mode::STEP_OFF_BAIL, Some(40)));
        assert_eq!(step_off(&mut c, &s, true, false, false, 40), None);
        // Online: straight to the wipe out.
        let mut c = ControllerState::default();
        assert_eq!(step_off(&mut c, &s, true, false, true, 40), Some(1.0));
    }

    #[test]
    fn the_reposition_node_follows_the_retail_choice() {
        let s = ControllerSettings::default();
        let step3 = ControllerState { sub_mode: sub_mode::STEP_OFF, ..Default::default() };
        assert_eq!(reposition_node(&step3, &s, true, 10, 12, 100), 17);
        assert_eq!(reposition_node(&step3, &s, true, 10, 30, 100), 32);
        assert_eq!(reposition_node(&step3, &s, true, 10, 30, 20), 19);
        let other = ControllerState { sub_mode: sub_mode::NORMAL, remembered_node: Some(20), ..Default::default() };
        assert_eq!(reposition_node(&other, &s, false, 10, 0, 100), 22);
        let far = ControllerState { remembered_node: Some(40), ..other };
        assert_eq!(reposition_node(&far, &s, false, 10, 0, 100), 12);
    }

    #[test]
    fn the_reposition_is_one_shot_and_throttled() {
        let s = ControllerSettings::default();
        let mut c = ControllerState { sub_mode: sub_mode::STEP_OFF, reposition_request: true, ..Default::default() };
        assert_eq!(take_reposition(&mut c, &s, false, 100, true, 10, 12, 100), Some(17));
        reposition_done(&mut c);
        assert_eq!((c.sub_mode, c.last_reposition), (sub_mode::NORMAL, Some(100)));
        c.reposition_request = true;
        assert_eq!(take_reposition(&mut c, &s, false, 130, false, 10, 12, 100), None);
        assert!(!c.reposition_request);
        c.reposition_request = true;
        assert_eq!(take_reposition(&mut c, &s, false, 160, false, 10, 12, 100), Some(12));
        // Online: never.
        c.reposition_request = true;
        assert_eq!(take_reposition(&mut c, &s, true, 1000, false, 10, 12, 100), None);
    }

    #[test]
    fn mode_6_enters_low_prop_without_lowering_a_higher_sub_mode() {
        let mut c = ControllerState::default();
        enter_low_prop(&mut c);
        assert_eq!(c.sub_mode, sub_mode::LOW_PROP);
        let mut c = ControllerState { sub_mode: sub_mode::STEP_OFF_BAIL, ..Default::default() };
        enter_low_prop(&mut c);
        assert_eq!(c.sub_mode, sub_mode::STEP_OFF_BAIL);
    }

    fn node(frames: u8, event: u8, flags: u8, jump: Option<u32>, step: [f32; 3]) -> crate::living_world::replay::ReplayNode {
        crate::living_world::replay::ReplayNode { position: [0.0; 3], step, board: [128; 4], skater: [128; 4], frames, event, flags, jump, width: [50, 50] }
    }

    fn jump(trick: i16, flags: u8) -> crate::living_world::replay::ReplayJump {
        crate::living_world::replay::ReplayJump { start_position: [0.0; 3], start_velocity: [0.0; 3], offset: [0.0; 3], trick, spins: 0, flags }
    }

    #[test]
    fn the_safe_node_is_the_first_of_three_calm_nodes_past_tricks_flags_and_slopes() {
        use crate::living_world::replay::{node_events, ReplayLine};
        let s = ControllerSettings::default();
        let flat = [0.0, 0.0, 0.1];
        // 0: a jump marker (frames reset); 1..4: 10 frames each (40, not past 45); 5: a flag-0x4 node; 6..: calm.
        let mut nodes = vec![node(10, 0, 0, Some(0), flat)];
        nodes.extend((0..4).map(|_| node(10, 0, 0, None, flat)));
        nodes.push(node(10, 0, 0x4, None, flat));
        nodes.extend((0..6).map(|_| node(10, 0, 0, None, flat)));
        let line = ReplayLine { id: [0; 16], flags: 0, skill: -1, nodes, jumps: vec![jump(128, 1)], groups: Vec::new() };
        // Node 5 has jump frames 50 but flag frames 0; node 6: 60 / 10; node 7: 70 / 20 is the first good one.
        assert_eq!(safe_node(&line, 0, &s), 7);
        assert_eq!(safe_node(&line, 8, &s), 8);
        // A steep step (slope >= 1) breaks the run.
        let mut steep = line.clone();
        steep.nodes[8].step = [0.0, 0.2, 0.1];
        assert_eq!(safe_node(&steep, 0, &s), 9);
        // A trick started at node 7: an ollie (class 1) keeps every later node unsafe until its end event (none
        // here: the last node); a manual (class 8, nosemanual 2) is calm.
        let mut ollie = line.clone();
        ollie.jumps.push(jump(128, 0));
        ollie.nodes[7].event = node_events::START_TRICK;
        ollie.nodes[7].jump = Some(1);
        assert_eq!(safe_node(&ollie, 0, &s), 11);
        let mut manual = ollie.clone();
        manual.jumps[1].trick = 2;
        assert_eq!(safe_node(&manual, 0, &s), 7);
        let none = ReplayLine { nodes: vec![node(1, 0, 0x4, None, flat); 4], ..line.clone() };
        assert_eq!(safe_node(&none, 0, &s), 3);
    }

    #[test]
    fn the_line_controller_presses_the_board_toggle_every_five_ticks_until_back_on() {
        let mut t = OffBoardToggle::default();
        // A's activate: mode 0, no press on the first tick.
        assert!(!t.press(4, true, false));
        t.line_mode(false);
        let presses: Vec<bool> = (0..12).map(|_| { let p = t.press(4, true, false); t.line_mode(false); p }).collect();
        assert_eq!(presses.iter().filter(|&&p| p).count(), 2);
        assert!(presses[4] && presses[9], "the count passes 4 five ticks after the activate tick, then every 5: {presses:?}");
        // Not while toggling, not once on the board; mode 1 (stay off) presses only on the board.
        let mut busy = OffBoardToggle { mode: 2, ticks: 9 };
        assert!(!busy.press(4, true, true));
        let mut on = OffBoardToggle { mode: 2, ticks: 9 };
        assert!(!on.press(4, false, false));
        let mut stay = OffBoardToggle { mode: 1, ticks: 9 };
        assert!(stay.press(4, false, false));
    }

    #[test]
    fn a_bail_respawns_two_past_the_search_node_at_the_first_safe_node() {
        use crate::living_world::replay::ReplayLine;
        let s = ControllerSettings::default();
        let flat = [0.0, 0.0, 0.1];
        // Calm 60-frame nodes, except a jump marker at node 9 (frames reset there).
        let mut nodes: Vec<_> = (0..20).map(|_| node(60, 0, 0, None, flat)).collect();
        nodes[9].jump = Some(0);
        let line = ReplayLine { id: [0; 16], flags: 0, skill: -1, nodes, jumps: vec![jump(128, 1)], groups: Vec::new() };
        let state = ControllerState::default();
        assert_eq!(respawn_node(&state, &s, 4, &line), safe_node(&line, 6, &s));
        assert_eq!(respawn_node(&state, &s, 4, &line), 6);
        // Past the jump marker: not on the take-off.
        assert_eq!(respawn_node(&state, &s, 8, &line), safe_node(&line, 10, &s));
        assert!(respawn_node(&state, &s, 8, &line) > 9);
        // A remembered prop ahead within the window moves it on; clamped at the last node.
        let remembered = ControllerState { remembered_node: Some(12), ..Default::default() };
        assert_eq!(respawn_node(&remembered, &s, 4, &line), safe_node(&line, 14, &s));
        assert_eq!(respawn_node(&state, &s, 19, &line), safe_node(&line, 19, &s));
    }
}
