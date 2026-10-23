//! The NPC skater's AI physics record (AIPhysicsInput, 164 bytes) built from its recorded line
//! (doc 26, "Simulated NPC skaters"). The physics reads it in the ground states' board path
//! (`crate::riding::grounded::state::board_path`).
//!
//! Retail (TU3, evidence only; re-implemented): `sub_8246DB50` -> `sub_8246DE38` each tick:
//! - target frame = the path frame of the committed cursor node (`sub_82453AD0`: board orientation
//!   `+0x18`, turned about up on board-flipped nodes, `sub_82453A58`) at the node position; when its
//!   forward points away from the skater's (`dot < 0`), its Ri and At rows are negated (the line is
//!   ridden switch / fakie instead of turning round);
//! - target velocity = the node's per-frame displacement (`+0x0C`) x 60 (`0x8302EE08`), its length
//!   clamped to 99.9 (`0x822F94D4`); above 0.001 (`0x82063A48`) the speed shape `sub_82470830`
//!   may scale it (`0.3 * s * influence + 0.125 * s * speed factor`, applied when it changes the
//!   speed by more than 0.01; both inputs not decoded yet, 0 here = no change);
//! - flags `+160`: bit 25 = on-board steering (`pc+922`, seeded by `sub_8246DB50`); with it (`pc+922`) bit 31 always, bit 30 = A and C, bit 29 = B
//!   and C, bit 28 = B, where A = (state byte `14652` clear and `pc+933` clear), B = A and
//!   `pc+945` clear, C = `pc+944` clear or trick category (`pc+832`) 9; without it all of bits
//!   31..28 (holding the cached pose when the skater is more than 10 m off the path,
//!   `0x821963E4`, AI kind 0).
//! The record's other words (trajectory block +80..+144, bits 27..25) are not built yet.

use super::replay::rotate;
use super::Vec3;
use crate::animation::output::actor_packet::ExternalPhysicsInput;

/// `0x8302EE08`: recorded per-frame displacement to m/s.
pub const FRAMES_PER_SECOND: f32 = 60.0;
/// `0x822F94D4`.
pub const MAX_SPEED: f32 = 99.9;

/// The PathController state bytes the flags read (retail riding defaults: all clear, steering on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SteerState {
    /// `pc+922`: on-board steering.
    pub on_board_steering: bool,
    /// The skater component's byte `14652`.
    pub byte_14652: bool,
    pub byte_933: bool,
    pub byte_944: bool,
    pub byte_945: bool,
    /// `pc+832`: the current trick category.
    pub trick_category: i32,
}

impl Default for SteerState {
    fn default() -> Self {
        Self { on_board_steering: true, byte_14652: false, byte_933: false, byte_944: false, byte_945: false, trick_category: 0 }
    }
}

impl SteerState {
    /// The `+160` steering bits, and bit 25 = on-board steering (`sub_8246DB50` seeds the word
    /// with `pc+922 << 25`; the physics state selector sends a skater whose record steers without
    /// it to `PHYSICS_STATE_FOLLOW_PATH`).
    pub fn flags(&self) -> u32 {
        if !self.on_board_steering {
            return 0xF000_0000;
        }
        let on_board = 1 << 25;
        let a = !self.byte_14652 && !self.byte_933;
        let b = a && !self.byte_945;
        let c = !self.byte_944 || self.trick_category == 9;
        on_board | (1 << 31) | (u32::from(a && c) << 30) | (u32::from(b && c) << 29) | (u32::from(b) << 28)
    }
}

/// Where the line is now (the committed cursor node): position, path frame (unit quaternion
/// x y z w) and the recorded per-frame displacement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineTarget {
    pub position: Vec3,
    pub frame: [f32; 4],
    pub step: Vec3,
}

fn words(v: Vec3, w: f32) -> [u32; 4] {
    [v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), w.to_bits()]
}

/// `sub_8246DE38`: the record for `target`, steered by a skater whose forward is `forward`.
pub fn build(target: &LineTarget, forward: Vec3, state: &SteerState) -> ExternalPhysicsInput {
    let mut ri = rotate(target.frame, [1.0, 0.0, 0.0]);
    let up = rotate(target.frame, [0.0, 1.0, 0.0]);
    let mut at = rotate(target.frame, [0.0, 0.0, 1.0]);
    if at[0] * forward[0] + at[1] * forward[1] + at[2] * forward[2] < 0.0 {
        ri = ri.map(|x| -x);
        at = at.map(|x| -x);
    }
    let mut v = target.step.map(|x| x * FRAMES_PER_SECOND);
    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if speed > MAX_SPEED {
        v = v.map(|x| x / speed * MAX_SPEED);
    }
    let mut vectors = [[0u32; 4]; 10];
    vectors[0] = words(ri, 0.0);
    vectors[1] = words(up, 0.0);
    vectors[2] = words(at, 0.0);
    vectors[3] = words(target.position, 1.0);
    vectors[4] = words(v, 0.0);
    // +128: the gravity / trajectory scalar, -1.0 splat without a trajectory (`0x8216DEE0`).
    vectors[8] = [(-1.0f32).to_bits(); 4];
    ExternalPhysicsInput { vectors, flags: state.flags() }
}

/// `sub_8246DB50` when it skips `sub_8246DE38`: the zeroed record with the -1 splat at +128 and the
/// flag word seeded with `pc+922 << 25`, bits 31..26 cleared. No steer bit, so the physics board path
/// does not run and the skater coasts on its own physics until the projection commits again.
pub fn without_steering(state: &SteerState) -> ExternalPhysicsInput {
    let mut vectors = [[0u32; 4]; 10];
    vectors[8] = [(-1.0f32).to_bits(); 4];
    ExternalPhysicsInput { vectors, flags: u32::from(state.on_board_steering) << 25 }
}

/// `sub_8246DB50`'s gate for `sub_8246DE38` (main checked 0x8246DCAC..0x8246DD1C): the steering record is
/// built when the skater is on the path and past the start nodes (`pc+920`, `pc+937`), or always without
/// on-board steering (`pc+922` clear). Not modelled yet: actor `1904` bit 26 and motion state 104, which
/// also skip it.
pub fn steers(state: &SteerState, on_path: bool, past_start: bool) -> bool {
    !state.on_board_steering || (on_path && past_start)
}

/// The spawn push (`sub_824701F8` hold branch): while the skater is still within 0.01 m
/// (`0x820D71E8`; 0.05 m `0x82165A00` when the cursor's node `pc+816` is not 0) of its node in the
/// ground plane, every board part gets the node's per-frame displacement x 60 x 0.75
/// (`0x821814A0`; x 0.5 `0x8209975C` for AI kind 1 outside motion states 19 / 20) as its velocity
/// (`82C04168`). `None` once it has left the node.
pub fn spawn_push(skater: Vec3, node: Vec3, step: Vec3, first_node: bool, ai_kind_1_slow: bool) -> Option<Vec3> {
    let limit: f32 = if first_node { 0.01 } else { 0.05 };
    let (dx, dz) = (skater[0] - node[0], skater[2] - node[2]);
    if dx * dx + dz * dz >= limit * limit {
        return None;
    }
    let scale = if ai_kind_1_slow { 0.5 } else { 0.75 };
    Some(step.map(|x| x * FRAMES_PER_SECOND * scale))
}

/// Gravity of a recorded jump's arc (`0x822F8B40`, the player's value).
pub const TRAJECTORY_GRAVITY: f32 = -9.8;
/// Record flag bits (`+160`): 26 = carries a recorded trajectory, 27 = it lands in a grind / slide.
pub const FLAG_TRAJECTORY: u32 = 1 << 26;
pub const FLAG_LANDS_IN_GRIND: u32 = 1 << 27;
/// The ext data's HasTrajectory bit (ext byte `+39` bit 0).
pub const EXT_HAS_TRAJECTORY: u8 = 1;

/// `sub_8246DA18` / `sub_82453870`: put a recorded jump into the record: +80 start position, +96
/// start velocity, +112 gravity (0, -9.8, 0), +128 the -1 splat, +144 the recorded offset; bit 26,
/// and bit 27 when it lands in a grind or slide ([`lands_in_grind`]). Nothing without the
/// HasTrajectory bit.
pub fn with_trajectory(record: &mut ExternalPhysicsInput, jump: &super::replay::ReplayJump, lands_in_grind: bool) {
    if jump.flags & EXT_HAS_TRAJECTORY == 0 {
        return;
    }
    record.vectors[5] = words(jump.start_position, 0.0);
    record.vectors[6] = words(jump.start_velocity, 0.0);
    record.vectors[7] = words([0.0, TRAJECTORY_GRAVITY, 0.0], 0.0);
    record.vectors[8] = [(-1.0f32).to_bits(); 4];
    record.vectors[9] = words(jump.offset, 0.0);
    record.flags |= FLAG_TRAJECTORY;
    if lands_in_grind {
        record.flags |= FLAG_LANDS_IN_GRIND;
    }
}

/// The node whose ext the record reads (`sub_8246DA18`, `pc+692`): the cursor's node while its
/// segment fraction is below 0.5, else the next node; the last node reads its own (b294).
pub fn trajectory_node(line: &super::replay::ReplayLine, node: u32, frame_in_segment: u32) -> u32 {
    if node as usize + 1 >= line.nodes.len() {
        return node;
    }
    let seg = line.segment_frames(node);
    if seg > 0 && frame_in_segment as f32 / seg as f32 >= 0.5 { node + 1 } else { node }
}

/// The record's trajectory this tick: the ext of [`trajectory_node`], only with HasTrajectory
/// (ext byte `+0x27` bit 0). Between jumps there is none, and the gate reads the cached previous
/// jump (`publication.rs`), as retail does. Bit 27 comes from `lands_in_grind(line, max(node - 1, 0))`
/// with the cursor's node (`pc+704`), not the ext's.
pub fn current_trajectory(line: &super::replay::ReplayLine, node: u32, frame_in_segment: u32) -> Option<&super::replay::ReplayJump> {
    let n = line.nodes.get(trajectory_node(line, node, frame_in_segment) as usize)?;
    n.jump.and_then(|j| line.jumps.get(j as usize)).filter(|j| j.flags & EXT_HAS_TRAJECTORY != 0)
}

/// `sub_824551C0(path, node)`: whether the recorded jump after `from` lands in a grind or slide.
/// From the first node carrying a trajectory on, it walks the line summing the recorded frames:
/// a start-trick of category 5 (grind / slide, `catalog::category`, `sub_824545F0`) answers yes;
/// category 8 / 9 (manual / powerslide), incidental air, an end-trick on the ground, another
/// trajectory more than 60 frames later or more than 600 frames in all answer no; after an
/// airborne end-trick the first ground node answers yes only when the next node starts a
/// category-5 trick.
pub fn lands_in_grind(line: &super::replay::ReplayLine, from: u32) -> bool {
    use super::replay::{node_events, node_flags};
    let category = |n: &super::replay::ReplayNode| crate::scoring::catalog::category(line.node_trick(n)).unwrap_or(0);
    let has_trajectory = |n: &super::replay::ReplayNode| n.jump.and_then(|j| line.jumps.get(j as usize)).is_some_and(|j| j.flags & EXT_HAS_TRAJECTORY != 0);
    let (mut seen, mut frames, mut landed_from_air) = (false, 0u32, false);
    for i in from as usize..line.nodes.len() {
        let n = &line.nodes[i];
        if !seen {
            seen = has_trajectory(n);
            continue;
        }
        frames += u32::from(n.frames);
        if has_trajectory(n) {
            if frames > 60 {
                return false;
            }
            frames = 0;
        }
        match n.event {
            node_events::START_TRICK => match category(n) {
                5 => return true,
                8 | 9 => return false,
                _ => {}
            },
            node_events::END_TRICK => {
                if n.flags & node_flags::AIRBORNE == 0 {
                    return false;
                }
                landed_from_air = true;
            }
            node_events::INCIDENTAL_AIR => return false,
            _ => {}
        }
        if landed_from_air && n.flags & node_flags::AIRBORNE == 0 {
            return line.nodes.get(i + 1).is_some_and(|next| next.event == node_events::START_TRICK && category(next) == 5);
        }
        if frames > 600 {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riding::grounded::state::board_path::{flags, SteerTarget};

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    use crate::living_world::replay::{node_events as ev, node_flags as nf, ReplayJump, ReplayLine, ReplayNode};

    /// A line from (event, flags, frames, jump) rows; jump 0 = the recorded trajectory, 1 = a
    /// grind slot (5050), 2 = a manual slot (nosemanual), 3 = a flip slot (360flip).
    fn line(rows: &[(u8, u8, u8, Option<u32>)]) -> ReplayLine {
        let jump = |trick: i16, flags: u8| ReplayJump { start_position: [1.0, 2.0, 3.0], start_velocity: [0.0, 4.0, 8.0], offset: [0.0, 0.1, 0.0], trick, spins: 0, flags };
        ReplayLine {
            id: [0; 16],
            flags: 4,
            skill: 0,
            nodes: rows
                .iter()
                .enumerate()
                .map(|(i, &(event, flags, frames, jump))| ReplayNode { position: [0.0, 0.0, i as f32], step: [0.0; 3], board: [128, 128, 128, 255], skater: [128, 128, 128, 255], frames, event, flags, jump, width: [50, 50] })
                .collect(),
            jumps: vec![jump(1, EXT_HAS_TRAJECTORY), jump(39, 0), jump(2, 0), jump(85, 0)],
            groups: vec![],
        }
    }

    #[test]
    fn a_recorded_jump_lands_in_a_grind_only_per_retails_scan() {
        let take_off = (0, 0, 4, Some(0));
        let air = (0, nf::AIRBORNE, 4, None);
        let ground = (0, 0, 4, None);
        // Straight into a grind.
        assert!(lands_in_grind(&line(&[ground, take_off, air, (ev::START_TRICK, nf::AIRBORNE, 4, Some(1))]), 0));
        // Into a manual: no.
        assert!(!lands_in_grind(&line(&[ground, take_off, air, (ev::START_TRICK, 0, 4, Some(2))]), 0));
        // A flip ending in the air, the first ground node, then a grind starts: yes; a flip after it: no.
        let flip = (ev::START_TRICK, nf::AIRBORNE, 4, Some(3));
        let end = (ev::END_TRICK, nf::AIRBORNE, 4, None);
        assert!(lands_in_grind(&line(&[take_off, flip, end, ground, (ev::START_TRICK, 0, 4, Some(1))]), 0));
        assert!(!lands_in_grind(&line(&[take_off, flip, end, ground, (ev::START_TRICK, 0, 4, Some(3))]), 0));
        // An end-trick on the ground, incidental air: no.
        assert!(!lands_in_grind(&line(&[take_off, flip, (ev::END_TRICK, 0, 4, None)]), 0));
        assert!(!lands_in_grind(&line(&[take_off, (ev::INCIDENTAL_AIR, nf::AIRBORNE, 4, None), (ev::START_TRICK, 0, 4, Some(1))]), 0));
        // More than 600 recorded frames before the grind: no; nothing before the trajectory counts.
        let mut rows = vec![take_off];
        rows.extend(std::iter::repeat_n((0, 0, 60, None), 11));
        rows.push((ev::START_TRICK, 0, 4, Some(1)));
        assert!(!lands_in_grind(&line(&rows), 0));
        assert!(!lands_in_grind(&line(&[(ev::START_TRICK, 0, 4, Some(1)), ground]), 0));
    }

    #[test]
    fn a_recorded_trajectory_fills_the_record_block() {
        let l = line(&[(0, 0, 4, Some(0))]);
        let mut r = build(&LineTarget { position: [0.0; 3], frame: IDENTITY, step: [0.0, 0.0, 0.1] }, [0.0, 0.0, 1.0], &SteerState::default());
        let base = r.flags;
        with_trajectory(&mut r, &l.jumps[0], true);
        let f = |i: usize| r.vectors[i].map(f32::from_bits);
        assert_eq!(f(5)[..3], [1.0, 2.0, 3.0]);
        assert_eq!(f(6)[..3], [0.0, 4.0, 8.0]);
        assert_eq!(f(7)[..3], [0.0, -9.8, 0.0]);
        assert_eq!(f(8), [-1.0; 4]);
        assert_eq!(f(9)[..3], [0.0, 0.1, 0.0]);
        assert_eq!(r.flags, base | FLAG_TRAJECTORY | FLAG_LANDS_IN_GRIND);
        // Only the cursor's nearest node (the next one from half the segment on), none between
        // jumps, the last node its own (b294, `sub_8246DA18` reads `pc+692`).
        let l2 = line(&[(0, 0, 4, None), (0, 0, 4, Some(0)), (0, nf::AIRBORNE, 4, None), (0, 0, 4, Some(0))]);
        assert!(current_trajectory(&l2, 0, 0).is_none());
        assert!(current_trajectory(&l2, 0, 1).is_none());
        assert_eq!(trajectory_node(&l2, 0, 2), 1);
        assert!(current_trajectory(&l2, 0, 2).is_some());
        assert!(current_trajectory(&l2, 1, 1).is_some());
        assert!(current_trajectory(&l2, 2, 0).is_none());
        assert_eq!(trajectory_node(&l2, 3, 0), 3);
        assert!(current_trajectory(&l2, 3, 0).is_some());
        // A slot without HasTrajectory adds nothing.
        let mut plain = build(&LineTarget { position: [0.0; 3], frame: IDENTITY, step: [0.0, 0.0, 0.1] }, [0.0, 0.0, 1.0], &SteerState::default());
        let before = plain.clone();
        with_trajectory(&mut plain, &l.jumps[1], true);
        assert_eq!(plain, before);
    }

    #[test]
    fn riding_defaults_steer_on_every_axis() {
        const ON_BOARD: u32 = 1 << 25;
        assert_eq!(SteerState::default().flags(), 0xF000_0000 | ON_BOARD);
        // 945 clears B (bits 29 and 28).
        let s = SteerState { byte_945: true, ..SteerState::default() };
        assert_eq!(s.flags(), ON_BOARD | (1 << 31) | (1 << 30));
        // 944 without category 9 clears bits 30 / 29; category 9 keeps them.
        let s = SteerState { byte_944: true, ..SteerState::default() };
        assert_eq!(s.flags(), ON_BOARD | (1 << 31) | (1 << 28));
        assert_eq!(SteerState { trick_category: 9, ..s }.flags(), 0xF000_0000 | ON_BOARD);
        assert_eq!(SteerState { byte_933: true, ..SteerState::default() }.flags(), ON_BOARD | 1 << 31);
        assert_eq!(SteerState { on_board_steering: false, byte_933: true, ..SteerState::default() }.flags(), 0xF000_0000);
    }

    #[test]
    fn the_spawn_push_starts_near_line_speed_only_on_the_node() {
        let v = spawn_push([0.0; 3], [0.005, 9.0, 0.0], [0.0, 0.0, 0.1], true, false).unwrap();
        assert!((v[2] - 4.5).abs() < 1e-5, "0.1 m/frame x 60 x 0.75");
        assert!((spawn_push([0.0; 3], [0.0; 3], [0.0, 0.0, 0.1], true, true).unwrap()[2] - 3.0).abs() < 1e-5);
        assert_eq!(spawn_push([0.02, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 0.1], true, false), None);
        assert!(spawn_push([0.02, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 0.1], false, false).is_some());
    }

    #[test]
    fn the_record_targets_the_line_pose_and_speed() {
        let t = LineTarget { position: [1.0, 2.0, 3.0], frame: IDENTITY, step: [0.0, 0.0, 0.1] };
        let r = build(&t, [0.0, 0.0, 1.0], &SteerState::default());
        let s = SteerTarget::from_words(&r.vectors, r.flags);
        assert_eq!((s.position.x, s.position.y, s.position.z), (1.0, 2.0, 3.0));
        assert!((s.velocity.z - 6.0).abs() < 1e-5, "0.1 m per frame = 6 m/s");
        assert_eq!((s.forward.x, s.forward.z), (0.0, 1.0));
        assert_ne!(s.flags & flags::STEER, 0);
        // Facing away: the frame's Ri / At flip, the velocity keeps the line's direction.
        let r = build(&t, [0.0, 0.0, -1.0], &SteerState::default());
        let s = SteerTarget::from_words(&r.vectors, r.flags);
        assert_eq!(s.forward.z, -1.0);
        assert!(s.velocity.z > 0.0);
        // Clamped at 99.9 m/s.
        let r = build(&LineTarget { step: [0.0, 0.0, 10.0], ..t }, [0.0, 0.0, 1.0], &SteerState::default());
        assert!((SteerTarget::from_words(&r.vectors, r.flags).velocity.z - MAX_SPEED).abs() < 1e-3);
    }

    #[test]
    fn off_the_path_or_before_the_start_nodes_the_record_does_not_steer() {
        let on_board = SteerState::default();
        assert!(steers(&on_board, true, true));
        assert!(!steers(&on_board, false, true), "off the path");
        assert!(!steers(&on_board, true, false), "before node 3");
        let walking = SteerState { on_board_steering: false, ..on_board };
        assert!(steers(&walking, false, false));
        let r = without_steering(&on_board);
        assert_eq!(r.flags, 1 << 25);
        assert_eq!(r.flags & (flags::STEER | flags::POSITION | flags::FACING | flags::VELOCITY), 0);
        assert_eq!(r.vectors[3], [0; 4], "no target position");
        assert_eq!(r.vectors[8].map(f32::from_bits), [-1.0; 4]);
        assert_eq!(without_steering(&walking).flags, 0);
    }
}
