//! NPC skaters, replay tier: the line cursor, retail branch choice (8246BEE0) and line chaining (8246C7F8).
//! Details: docs/hails-additions/26c-living-world-npc-skaters.md#line-cursor-model

use super::Vec3;
use std::collections::BTreeMap;

/// Recording rate of the lines [data].
pub const RECORDING_HZ: f64 = 60.0;

/// Node flag bits (`m_IsBoardFlipped`, `m_IsCrouched`, `m_IsAirborne`, `m_IsOffBoard`; bit order
/// as in `skate-data::aipath::node_flags`).
pub mod node_flags {
    pub const BOARD_FLIPPED: u8 = 1 << 0;
    pub const CROUCHED: u8 = 1 << 1;
    pub const AIRBORNE: u8 = 1 << 2;
    pub const OFF_BOARD: u8 = 1 << 3;
}

/// Node events (`m_EventType`).
pub mod node_events {
    pub const NONE: u8 = 0;
    pub const START_TRICK: u8 = 1;
    pub const END_TRICK: u8 = 2;
    pub const INCIDENTAL_AIR: u8 = 4;
}

/// Retail constants of the branch choice [code].
pub mod retail {
    /// `sub_8246C3C0`: reject at or above this angle (rad), `0x822F91B0`.
    pub const BRANCH_MAX_ANGLE: f32 = 0.872_665;
    /// Angle to score (tenths of a degree), `0x822F9354`.
    pub const BRANCH_ANGLE_SCALE: f32 = 572.958;
    /// Speed factor, `0x822F95F0` (-400, subtracted).
    pub const BRANCH_SPEED_SCALE: f32 = -400.0;
    /// Per other AI skater on the same line within [`BRANCH_CROWD_NODES`] (`rlwinm r29,r3,10`).
    pub const BRANCH_CROWD: i64 = 1024;
    pub const BRANCH_CROWD_NODES: u32 = 5;
    /// Player distance term: x 30 (`0x820D4924`), capped at 1500.
    pub const BRANCH_NEAR_SCALE: f32 = 30.0;
    pub const BRANCH_NEAR_CAP: i64 = 1500;
    /// Lines with flags bits 0..2 all set.
    pub const BRANCH_ALL_TYPES: i64 = 1000;
    /// Skill difference: x 250 + 100.
    pub const BRANCH_SKILL_SCALE: i64 = 250;
    pub const BRANCH_SKILL_BASE: i64 = 100;
    /// A taken branch searches `target - 3 ..= target` for the nearest node.
    pub const BRANCH_REJOIN_BACK: u32 = 3;
    /// Line end: radius around the end position for other lines' start nodes ([data]
    /// `ai_skater` `default` field `Hash_4F87E7A70DA11691` = 4.0, read by `sub_8246C7F8`).
    pub const CHAIN_RADIUS: f32 = 4.0;
    /// Line end: at most this many candidates (`sub_8246C7F8` passes 16 to `sub_82458968`).
    pub const CHAIN_MAX_CANDIDATES: usize = 16;
    /// Line end fallback (`sub_82458968` mode 1): the nearest valid start wins over a nearer
    /// invalid one only while its squared distance is below this (`0x822F94F4` = 36).
    pub const CHAIN_FALLBACK_VALID_D2: f32 = 36.0;
    /// AI steer input (`sub_8246B358` -> `sub_82471188`): no steer below this yaw error (deg),
    /// [data] `ai_skater` default tunable `DD8843F793462295` = 2.0.
    pub const STEER_DEAD_ZONE_DEG: f32 = 2.0;
    /// Full steer at this yaw error (deg), [data] `ai_skater` default tunable `281F55D7BB965ADC` = 10.0.
    pub const STEER_FULL_DEG: f32 = 10.0;
    /// The riding-fakie rule's thresholds (`UpdateRidingFakie82BB2330`, [`LineCursor::fakie`]):
    /// the stock motion graph's `UpdateRidingFakie` node [data] (checked by the data-gated
    /// `living_world_npc_fakie_rule_and_channel_match_the_stock_graph`).
    pub const FAKIE: crate::animation::riding_fakie::Settings = crate::animation::riding_fakie::Settings {
        high_speed: 1.0,
        low_speed: 0.5,
        slowly_backwards_seconds: 0.2,
        after_teleport_seconds: 1.0,
    };
    /// The fakie channel's fade in and out (s): `FakieHeadChannel82BAC778` blend in / out
    /// `0x3e99999a` = 0.3 [code].
    pub const FAKIE_CHANNEL_FADE_SECONDS: f32 = 0.3;
}

/// Which way the replay draws the NPC skater's body (the puppet root orientation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FacingRule {
    /// Retail (default): recorded skater frame slerped between nodes, turned while the latched flip is set: docs/hails-additions/26c-living-world-npc-skaters.md#facing-rule-riding-entry
    #[default]
    RidingEntry,
    /// NOT RETAIL, mod option (`per_node`, the fix 23 rule): per node, the recorded skater frame
    /// turned 180 deg wherever it faces more than 90 deg away from the node's path frame
    /// ([`drawn_skater`]); no state. A switch or fakie stretch is drawn riding forward.
    PerNode,
}

impl FacingRule {
    /// Stable mod-facing names (`skater_line_chain.facing_rule`).
    pub const NAMES: [&'static str; 2] = ["riding_entry", "per_node"];
    pub fn name(self) -> &'static str {
        match self {
            FacingRule::RidingEntry => "riding_entry",
            FacingRule::PerNode => "per_node",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "riding_entry" => Some(FacingRule::RidingEntry),
            "per_node" => Some(FacingRule::PerNode),
            _ => None,
        }
    }
}

/// Retail AI steer input from a signed yaw error (824536C8, 82471188): dead zone, linear to full; simulated tier only.
pub fn steer_input(yaw_error: f32, dead_zone_deg: f32, full_deg: f32) -> f32 {
    let e = yaw_error.abs().to_degrees();
    let span = full_deg - dead_zone_deg;
    let m = if span > 0.0 { ((e - dead_zone_deg) / span).clamp(0.0, 1.0) } else if e > dead_zone_deg { 1.0 } else { 0.0 };
    -m * yaw_error.signum()
}

/// How a skater continues at the end of its line (retail `sub_8246D3C0` -> `sub_8246C7F8`):
/// data-driven, retail values by default, a mod may change them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainConfig {
    /// Look for other lines whose start node lies within this many metres of the end position
    /// (retail 4.0). 0 or less: never chain (the skater fades out at its line end).
    pub radius: f32,
    /// Candidate cap (retail 16).
    pub max_candidates: usize,
    /// No start within `radius`: nearest unused start at any distance (82458968 mode 1, b122); physics tier only.
    pub fallback: bool,
    pub fallback_valid_d2: f32,
    /// Seconds the drawn root blends onto the new line after a branch or chain (not retail; 0 = cut): docs/hails-additions/26c-living-world-npc-skaters.md#switch-blend-seconds
    pub blend_seconds: f32,
    /// Mod option, not retail (default false): ride the new line turned 180 deg after a switch (fix 16): docs/hails-additions/26c-living-world-npc-skaters.md#keep-facing-option
    pub keep_facing: bool,
    /// How the body is drawn ([`FacingRule`]; default and retail [`FacingRule::RidingEntry`]; the
    /// fix 23 [`FacingRule::PerNode`] is a mod option).
    pub facing_rule: FacingRule,
    /// Retail AI steer ramp ([`steer_input`]; `ai_skater` defaults 2 / 10 deg). Data for the
    /// simulated tier; the replay tier does not steer.
    pub steer_dead_zone_deg: f32,
    pub steer_full_deg: f32,
    /// Retail riding-fakie rule thresholds ([`LineCursor::fakie`], [`retail::FAKIE`]).
    pub fakie: crate::animation::riding_fakie::Settings,
}

/// Default switch blend (engine stand-in, see [`ChainConfig::blend_seconds`]): the stock motion
/// graph's default transition time (literal `0x82099280` = 0.2 s [code]), the time the puppet
/// crossfades its clips over, so root and pose settle together.
pub const SWITCH_BLEND_SECONDS: f32 = 0.2;

impl ChainConfig {
    /// Retail values, except the engine `blend_seconds`.
    pub fn retail() -> Self {
        Self {
            radius: retail::CHAIN_RADIUS,
            max_candidates: retail::CHAIN_MAX_CANDIDATES,
            fallback: false,
            fallback_valid_d2: retail::CHAIN_FALLBACK_VALID_D2,
            blend_seconds: SWITCH_BLEND_SECONDS,
            keep_facing: false,
            facing_rule: FacingRule::RidingEntry,
            steer_dead_zone_deg: retail::STEER_DEAD_ZONE_DEG,
            steer_full_deg: retail::STEER_FULL_DEG,
            fakie: retail::FAKIE,
        }
    }
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self::retail()
    }
}

/// One recorded node (`tAIPathNode`, 44 bytes on the disc).
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayNode {
    pub position: Vec3,
    /// Recorded displacement per 60 Hz frame (node `+0x0C`, "direction"; measured on the export:
    /// |step| x frames = the segment length, median ratio 1.000). The AI record's target velocity
    /// is this x 60 ([code] `sub_8246DE38`).
    pub step: Vec3,
    /// Board and skater orientation, 4 biased bytes each (`(b - 128) / 127`, x y z w).
    pub board: [u8; 4],
    pub skater: [u8; 4],
    /// 60 Hz frames since the previous node.
    pub frames: u8,
    pub event: u8,
    pub flags: u8,
    /// Index into [`ReplayLine::jumps`].
    pub jump: Option<u32>,
    /// Path width left / right of the line, node bytes `+0x25/+0x26`; metres = byte / 50
    /// ([`WIDTH_SCALE`]). The obstacle avoider reads them ([`super::avoid`]).
    pub width: [u8; 2],
}

/// Node width byte to metres: `1 / 50` (`sub_82F71580`: 1.0 / `0x8302EE0C`).
pub const WIDTH_SCALE: f32 = 1.0 / 50.0;

/// A recorded jump / trick slot (`tAIPathNodeExtData`).
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayJump {
    pub start_position: Vec3,
    pub start_velocity: Vec3,
    pub offset: Vec3,
    pub trick: i16,
    pub spins: i8,
    pub flags: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayBranch {
    pub target: [u8; 16],
    pub target_node: u32,
    /// The disc's f32 (0..1); not read by the branch choice [code].
    pub weight: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayBranchGroup {
    pub node: u32,
    pub branches: Vec<ReplayBranch>,
}

/// One recorded line as the cursor needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayLine {
    pub id: [u8; 16],
    pub flags: u32,
    /// `m_SkillLevel` (path +80); -1 = none.
    pub skill: i32,
    pub nodes: Vec<ReplayNode>,
    pub jumps: Vec<ReplayJump>,
    pub groups: Vec<ReplayBranchGroup>,
}

impl ReplayLine {
    pub fn group_at(&self, node: u32) -> Option<&ReplayBranchGroup> {
        self.groups.iter().find(|g| g.node == node)
    }
    /// 60 Hz frames from node `i` to node `i + 1`.
    pub fn segment_frames(&self, i: u32) -> u32 {
        self.nodes.get(i as usize + 1).map_or(0, |n| u32::from(n.frames))
    }
    pub fn duration_frames(&self) -> u64 {
        self.nodes.iter().skip(1).map(|n| u64::from(n.frames)).sum()
    }
    /// Whether a trick span (START_TRICK without its END_TRICK yet) is open at `node`.
    pub fn trick_open_at(&self, node: u32) -> bool {
        self.open_trick_at(node).is_some()
    }
    /// The recorded trick of the span open at `node`: `Some(id)` while a span is open (`-1` when
    /// its START_TRICK node has no trick slot), `None` outside a span.
    pub fn open_trick_at(&self, node: u32) -> Option<i16> {
        self.nodes[..=(node as usize).min(self.nodes.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|n| match n.event {
                node_events::START_TRICK => Some(Some(self.node_trick(n))),
                node_events::END_TRICK => Some(None),
                _ => None,
            })
            .flatten()
    }
    /// The trick id of a node's trick slot (`tAIPathNodeExtData` +0x24, i16, an `EScorableID`
    /// valid in 0..332: [code] `sub_8246A2E0` at 0x8246A4A0 reads `lhz 36(ext)`, `extsh`, checks
    /// `> -1` and `< 332`), `-1` without a slot.
    pub fn node_trick(&self, n: &ReplayNode) -> i16 {
        n.jump.and_then(|j| self.jumps.get(j as usize)).map_or(-1, |j| j.trick)
    }
}

/// Where the cursor finds lines by id (the loaded district, a mod's lines).
pub trait LineSource {
    fn line(&self, id: &[u8; 16]) -> Option<&ReplayLine>;
    /// Every loaded line in a stable order (sorted by id), for the line-end search.
    fn for_each_line<'a>(&'a self, f: &mut dyn FnMut(&'a ReplayLine));
}

impl LineSource for BTreeMap<[u8; 16], ReplayLine> {
    fn line(&self, id: &[u8; 16]) -> Option<&ReplayLine> {
        self.get(id)
    }
    fn for_each_line<'a>(&'a self, f: &mut dyn FnMut(&'a ReplayLine)) {
        for l in self.values() {
            f(l);
        }
    }
}

/// Decode a node orientation: x, y, z, w with +Z = forward ([data]: the skater quaternion turns
/// +Z onto the travel direction within 25 deg on 81 % of moving nodes in this order; other
/// orders and axes score far lower).
pub fn decode_orientation(raw: [u8; 4]) -> [f32; 4] {
    let q = raw.map(|b| (f32::from(b) - 128.0) / 127.0);
    let n = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if n > 1e-6 {
        q.map(|c| c / n)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Rotate `v` by the unit quaternion `q` (x, y, z, w).
pub fn rotate(q: [f32; 4], v: Vec3) -> Vec3 {
    let [x, y, z, w] = q;
    let t = [2.0 * (y * v[2] - z * v[1]), 2.0 * (z * v[0] - x * v[2]), 2.0 * (x * v[1] - y * v[0])];
    [v[0] + w * t[0] + (y * t[2] - z * t[1]), v[1] + w * t[1] + (z * t[0] - x * t[2]), v[2] + w * t[2] + (x * t[1] - y * t[0])]
}

/// Retail path frame of a node (82453A58): board orientation, turned 180 deg when `m_IsBoardFlipped`.
pub fn path_frame(node: &ReplayNode) -> [f32; 4] {
    let q = decode_orientation(node.board);
    if node.flags & node_flags::BOARD_FLIPPED != 0 { turn_about_up(q) } else { q }
}

/// The raw recorded pose at line position (node, t), t in [0, 1] along node -> node + 1: the steering target retail
/// builds from cursor B (`sub_8246DE38`, b110 2.7): position and path frame interpolated, the segment's per-frame
/// displacement (the next node's `step`). The last node gives its own pose.
pub fn line_target_at(line: &ReplayLine, at: (u32, f32)) -> Option<super::ai_record::LineTarget> {
    let a = line.nodes.get(at.0 as usize)?;
    Some(match line.nodes.get(at.0 as usize + 1) {
        Some(b) => segment_target(a, b, at.1.clamp(0.0, 1.0)),
        None => segment_target(a, a, 0.0),
    })
}

fn segment_target(a: &ReplayNode, b: &ReplayNode, t: f32) -> super::ai_record::LineTarget {
    let position = core::array::from_fn(|k| a.position[k] + (b.position[k] - a.position[k]) * t);
    super::ai_record::LineTarget { position, frame: nlerp(path_frame(a), path_frame(b), t), step: b.step }
}

/// The node world frame retail spawns the character with ([code] `sub_82453C58`, called by the
/// spawn `sub_8245C548` before `sub_8245DA78`): the recorded skater frame on off-board nodes
/// (flags bit 0x08), the path frame ([`path_frame`]) on every other node.
pub fn node_world_frame(node: &ReplayNode) -> [f32; 4] {
    if node.flags & node_flags::OFF_BOARD != 0 { decode_orientation(node.skater) } else { path_frame(node) }
}

/// Whether a node counts as riding on the board: neither airborne nor off board (the physical
/// category of [`LineCursor`]'s fakie rule, and the ground-spawn stand-in of the flip latch).
pub fn node_riding(flags: u8) -> bool {
    flags & (node_flags::AIRBORNE | node_flags::OFF_BOARD) == 0
}

/// Whether a node counts as airborne for the flip latch (82DB6EC0 `+438`, b119 / b354 / b355): docs/hails-additions/26c-living-world-npc-skaters.md#node-airborne
pub fn node_airborne(flags: u8) -> bool {
    flags & node_flags::AIRBORNE != 0 && flags & node_flags::OFF_BOARD == 0
}

/// Retail flip test ([code] `sub_8246A700`): the recorded skater frame of the current node faces
/// away from the character's forward, `dot(row 2, row 2) < 0` (3D, `vmsum3fp128`).
pub fn flip_test(node: &ReplayNode, character: [f32; 4]) -> bool {
    let a = rotate(decode_orientation(node.skater), [0.0, 0.0, 1.0]);
    let b = rotate(character, [0.0, 0.0, 1.0]);
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] < 0.0
}

/// Shortest-arc slerp of two unit quaternions (x, y, z, w), like [code] `sub_82454B28` (sign
/// selection by the dot product, normalised lerp when the two are close; the closeness threshold
/// is not decoded, 0.9995 here is ours).
pub fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let b = if dot < 0.0 {
        dot = -dot;
        b.map(|c| -c)
    } else {
        b
    };
    if dot > 0.9995 {
        return nlerp(a, b, t);
    }
    let theta = dot.min(1.0).acos();
    let s = theta.sin();
    let (wa, wb) = (((1.0 - t) * theta).sin() / s, (t * theta).sin() / s);
    std::array::from_fn(|i| a[i] * wa + b[i] * wb)
}

/// Retail target frame between nodes `i` and `j` at fraction `t` (controller `+208`): the
/// recorded skater frames slerped ([`slerp`]), before the flip.
pub fn target_skater(line: &ReplayLine, i: usize, j: usize, t: f32) -> [f32; 4] {
    let (Some(a), Some(b)) = (line.nodes.get(i), line.nodes.get(j)) else { return [0.0, 0.0, 0.0, 1.0] };
    slerp(decode_orientation(a.skater), decode_orientation(b.skater), t)
}

/// NOT RETAIL (fix 23 rule, `FacingRule::PerNode` mod option): draw node `i` turned 180 deg: docs/hails-additions/26c-living-world-npc-skaters.md#per-node-facing-rule
fn skater_turned(line: &ReplayLine, i: usize) -> bool {
    let mut k = i.min(line.nodes.len().saturating_sub(1));
    while k > 0 && line.nodes[k].flags & node_flags::AIRBORNE != 0 {
        k -= 1;
    }
    let Some(n) = line.nodes.get(k) else { return false };
    let f = rotate(decode_orientation(n.skater), [0.0, 0.0, 1.0]);
    let g = rotate(path_frame(n), [0.0, 0.0, 1.0]);
    f[0].hypot(f[2]) > 1e-3 && g[0].hypot(g[2]) > 1e-3 && yaw_angle(f, g).abs() > std::f32::consts::FRAC_PI_2
}

/// NOT RETAIL (`FacingRule::PerNode`): the skater orientation that rule draws at node `i`.
pub fn drawn_skater(line: &ReplayLine, i: usize) -> [f32; 4] {
    let Some(n) = line.nodes.get(i) else { return [0.0, 0.0, 0.0, 1.0] };
    let q = decode_orientation(n.skater);
    if skater_turned(line, i) { turn_about_up(q) } else { q }
}

/// Diagnostic thresholds for [`facing_check`] (engine constants for the `NPC_SKATER_BACKWARDS`
/// log, not retail values).
pub mod facing_diagnostic {
    /// Drawn heading this far (radians, 135 deg) from the velocity yaw counts as riding backwards.
    pub const BACKWARDS_ANGLE: f32 = 135.0 * std::f32::consts::PI / 180.0;
    /// Below this ground speed (m/s) the velocity yaw is too noisy to judge.
    pub const MIN_SPEED: f32 = 1.0;
}

/// Drawn heading against the direction of travel at one sample (the `NPC_SKATER_BACKWARDS` log).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FacingCheck {
    /// Yaw of the drawn skater's +Z about +Y (radians, 0 = +Z).
    pub heading_yaw: f32,
    /// Yaw of the velocity (radians).
    pub velocity_yaw: f32,
    /// |heading - velocity| wrapped to 0..pi.
    pub angle: f32,
    /// The line's own retail path frame ([`path_frame`]) faces away from the travel by more than
    /// 90 deg here: the recorder rode fakie, so retail's target frame opposes travel too.
    pub recorded_fakie: bool,
    /// The body is drawn riding fakie here ([`ReplaySample::fakie`]: retail's fakie bit, the
    /// fakie channel plays): a heading against travel with this set is retail's fakie drawing,
    /// not a skater riding backwards in a forward pose.
    pub drawn_fakie: bool,
    pub backwards: bool,
}

/// [`FacingCheck::recorded_fakie`] for travel along `velocity` at a node.
pub fn node_recorded_fakie(node: &ReplayNode, velocity: [f32; 3]) -> bool {
    let g = rotate(path_frame(node), [0.0, 0.0, 1.0]);
    g[0].hypot(g[2]) > 1e-3 && yaw_angle(g, velocity).abs() > std::f32::consts::FRAC_PI_2
}

/// [`FacingCheck`] for a sample of `line`; `None` when the ground speed is below
/// [`facing_diagnostic::MIN_SPEED`] or the heading is vertical.
pub fn facing_check(line: &ReplayLine, s: &ReplaySample) -> Option<FacingCheck> {
    let v = s.velocity;
    if v[0].hypot(v[2]) < facing_diagnostic::MIN_SPEED {
        return None;
    }
    let f = rotate(s.skater, [0.0, 0.0, 1.0]);
    if f[0].hypot(f[2]) < 1e-3 {
        return None;
    }
    let angle = yaw_angle(f, v).abs();
    let recorded_fakie = line.nodes.get(s.node as usize).is_some_and(|n| node_recorded_fakie(n, v));
    Some(FacingCheck { heading_yaw: f[0].atan2(f[2]), velocity_yaw: v[0].atan2(v[2]), angle, recorded_fakie, drawn_fakie: s.fakie, backwards: angle > facing_diagnostic::BACKWARDS_ANGLE })
}

/// `q` turned 180 deg about its own +Y: `q * (0, 1, 0, 0)` (x, y, z, w), i.e. the frame's X and Z
/// axes negated, as retail does for a board-flipped node ([code] `sub_82453A58`).
pub fn turn_about_up(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    [-z, w, x, -y]
}

fn nlerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let q: [f32; 4] = std::array::from_fn(|i| a[i] + (s * b[i] - a[i]) * t);
    let n = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if n > 1e-6 {
        q.map(|c| c / n)
    } else {
        a
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// What the NPC is doing at a node (puppet animation, audio).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReplayPhase {
    Rolling,
    Crouched,
    Air,
    /// Airborne inside a trick span (a flip / grab / spin slot).
    AirTrick,
    /// On the ground inside a trick span: a grind, slide or manual (the recording does not say
    /// which; open until the simulated tier performs the trick).
    GroundTrick,
    OffBoard,
}

impl ReplayPhase {
    pub fn of(flags: u8, trick_open: bool) -> Self {
        if flags & node_flags::OFF_BOARD != 0 {
            ReplayPhase::OffBoard
        } else if flags & node_flags::AIRBORNE != 0 {
            if trick_open {
                ReplayPhase::AirTrick
            } else {
                ReplayPhase::Air
            }
        } else if trick_open {
            ReplayPhase::GroundTrick
        } else if flags & node_flags::CROUCHED != 0 {
            ReplayPhase::Crouched
        } else {
            ReplayPhase::Rolling
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ReplayPhase::Rolling => "rolling",
            ReplayPhase::Crouched => "crouched",
            ReplayPhase::Air => "air",
            ReplayPhase::AirTrick => "air_trick",
            ReplayPhase::GroundTrick => "ground_trick",
            ReplayPhase::OffBoard => "off_board",
        }
    }
}

/// Phases the cursor remembers for the puppet's nested crossfade (current first). A clip change
/// while an earlier transition still runs blends out of the running blend, like a retail graph
/// transition whose outgoing tree is the previous transition ([code] Blend82B96058, fix 21).
pub const PHASE_HISTORY: usize = 6;

/// One phase the NPC entered: what, when (60 Hz frame since spawn) and the recorded trick of the
/// span it belongs to (`-1` = none or no slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseEntry {
    pub phase: ReplayPhase,
    pub since: u64,
    pub trick: i16,
    /// Frame the span's trick started (its slot tick 0, research b704): docs/hails-additions/26c-living-world-npc-skaters.md#npc-grab-tweaks
    pub trick_since: u64,
}

/// The NPC's state at one instant.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplaySample {
    pub line: [u8; 16],
    pub node: u32,
    pub position: Vec3,
    /// m/s, from the current segment.
    pub velocity: Vec3,
    /// Yaw about +Y (0 = +Z, the engine's forward).
    pub heading: f32,
    pub board: [f32; 4],
    pub skater: [f32; 4],
    pub flags: u8,
    pub phase: ReplayPhase,
    /// The recorded jump of the current node, if any.
    pub jump: Option<u32>,
    /// Frames spent in the current phase (60 Hz), for clip time.
    pub phase_frames: u64,
    /// The phase before the current one (`None` until the first change), for the puppet's
    /// crossfade out of its clip.
    pub previous_phase: Option<ReplayPhase>,
    /// Frames since the previous phase began (60 Hz): its clip keeps playing while it blends out,
    /// like the outgoing tree of a graph transition.
    pub previous_phase_frames: u64,
    /// The render fraction (0..1) of the next 60 Hz frame this sample was taken at: clip and blend
    /// times are `(frames + sub_frame) / 60`, so the pose moves with the root between ticks.
    pub sub_frame: f32,
    /// Retail's riding-fakie bit of the drawn body ([`LineCursor::fakie`]).
    pub fakie: bool,
}

/// The drawn root at a branch or chain: where the skater was drawn on the old line, decayed onto
/// the new line over [`LineCursor::switch_blend_seconds`] with the graph transition curve.
#[derive(Clone, Debug, PartialEq)]
pub struct LineSwitch {
    /// Cursor frame of the switch.
    pub frame: u64,
    /// Drawn position on the old line minus the new line's position at the switch.
    pub offset: Vec3,
    /// Drawn orientations at the switch (x, y, z, w).
    pub skater: [f32; 4],
    pub board: [f32; 4],
}

/// A branch decision (host side) or the record a client mirrors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRecord {
    /// Cursor frame (60 Hz frames since spawn) of the decision.
    pub frame: u64,
    pub from_line: [u8; 16],
    pub from_node: u32,
    pub to_line: [u8; 16],
    pub to_node: u32,
    /// A host restart on `to_line` / `to_node` (re-place, re-snap, bail respawn, walk-back hand back), not a branch:
    /// the client applies [`LineCursor::respawn`]'s spawn state instead of a switch.
    pub respawn: bool,
}

/// A trick choice at a start-trick node (host side) or the record a client mirrors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrickRecord {
    /// Cursor frame (60 Hz frames since spawn) of the node.
    pub frame: u64,
    pub line: [u8; 16],
    pub node: u32,
    /// The trick recorded on the line and the one started (`-1` = none).
    pub recorded: i16,
    pub chosen: i16,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CursorEvent {
    /// The cursor reached a node (event and flags of that node; mods and speech read these).
    Node { line: [u8; 16], node: u32, event: u8, flags: u8 },
    Branch(BranchRecord),
    /// The trick chosen at a start-trick node ([`super::npc_tricks::choose`]).
    Trick(TrickRecord),
    /// End of the line with no next line within the chain radius (replay tier only: retail's
    /// full skater would steer to the nearest start node at any distance).
    Finished,
}

/// What the branch score reads besides the lines [code `sub_8246C230`].
#[derive(Clone, Copy, Debug)]
pub struct BranchContext<'a> {
    /// The skater's position and forward (the NPC itself).
    pub position: Vec3,
    pub forward: Vec3,
    pub speed: f32,
    /// The players (the 1500 term uses the nearest; one observer = retail's local player).
    pub players: &'a [Vec3],
    /// Other AI skaters' (line, node) (the 1024 term).
    pub others: &'a [([u8; 16], u32)],
    /// Lines in use by other skaters (`sub_82458860`).
    pub in_use: &'a [[u8; 16]],
    /// The controller's preferred skill level (+164, -1 = none).
    pub preferred_skill: i32,
    /// Online (mgr+608): the crowd and player terms are skipped.
    pub online: bool,
    /// What happens at the end of a line ([`choose_next_line`]).
    pub chain: ChainConfig,
    /// The trick choice at start-trick nodes ([`super::npc_tricks`]).
    pub tricks: super::npc_tricks::TrickContext<'a>,
    /// A physics-simulated skater (retail's full skater): its gate and body point ([`BodyGate`]); `None` = the replay
    /// tier, which stands on the node it reached.
    pub body: Option<BodyGate>,
    /// The lines valid for this skater's character (`sub_82456970`, sorted), or `None` = every line (no profile).
    pub valid_lines: Option<&'a [[u8; 16]]>,
}

impl BranchContext<'_> {
    fn valid(&self, line: &[u8; 16]) -> bool {
        self.valid_lines.is_none_or(|v| v.binary_search(line).is_ok())
    }
}

/// The node-change handler's own inputs for a simulated skater (`sub_8246BEE0`, research b674).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyGate {
    /// The projected body point (`sub_82467FC8`): the branch score and the rejoin node are taken from it.
    pub point: Vec3,
    /// The handler's gate: on-board steering, not airborne (Air+0x1B6), trick category not 1 / 2 / 5 (perform, grind /
    /// slide). False = no branch this node.
    pub may_branch: bool,
    /// The heading the branch and line-end scores take (`sub_8246C3C0`): the effective deck forward (`82592A00`) and
    /// the skater frame's up row, the axis the angle is measured about (`824536C8`, research b706). `None`: the
    /// context's forward about world Y.
    pub heading: Option<(Vec3, Vec3)>,
}

/// The trick categories in which `sub_8246BEE0` takes no branch (b674; data a mod may change).
pub const NO_BRANCH_CATEGORIES: [u8; 3] = [1, 2, 5];

/// Signed angle from `a` to `b` about +Y (radians), like `sub_824536C8` with the up axis.
fn yaw_angle(a: Vec3, b: Vec3) -> f32 {
    let (ax, az, bx, bz) = (a[0], a[2], b[0], b[2]);
    let cross = az * bx - ax * bz;
    let dot = ax * bx + az * bz;
    cross.atan2(dot)
}

/// Signed angle from `a` to `b` about `axis` (unit), both projected onto the plane across it (`824536C8`).
fn angle_about(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    let flat = |v: Vec3| {
        let d = v[0] * axis[0] + v[1] * axis[1] + v[2] * axis[2];
        [v[0] - axis[0] * d, v[1] - axis[1] * d, v[2] - axis[2] * d]
    };
    let (a, b) = (flat(a), flat(b));
    let cross = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let sin = cross[0] * axis[0] + cross[1] * axis[1] + cross[2] * axis[2];
    let cos = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    sin.atan2(cos)
}

/// Score one (line, node) candidate; `None` = rejected. Lowest wins.
pub fn branch_score(line: &ReplayLine, node: u32, ctx: &BranchContext) -> Option<i64> {
    let count = line.nodes.len() as u32;
    // sub_8246C4F8: airborne or event nodes within one of the candidate node (skipped at node 0).
    if node > 0 {
        for i in node - 1..=node + 1 {
            if let Some(n) = line.nodes.get(i as usize) {
                if n.flags & node_flags::AIRBORNE != 0 || n.event != 0 {
                    return None;
                }
            }
        }
    }
    // sub_8246C3C0: the next node must exist and lie within 50 deg of the forward.
    let next = line.nodes.get(node as usize + 1)?;
    let to_next = sub(next.position, ctx.position);
    let angle = match ctx.body.and_then(|b| b.heading) {
        Some((forward, up)) => angle_about(forward, to_next, up),
        None => yaw_angle(ctx.forward, to_next),
    }
    .abs();
    if !(angle < retail::BRANCH_MAX_ANGLE) {
        return None;
    }
    let mut score = (angle * retail::BRANCH_ANGLE_SCALE) as i64;
    score -= (ctx.speed * retail::BRANCH_SPEED_SCALE) as i64;
    if !ctx.online {
        let crowd = ctx.others.iter().filter(|(l, n)| *l == line.id && n.abs_diff(node) <= retail::BRANCH_CROWD_NODES).count() as i64;
        score += crowd * retail::BRANCH_CROWD;
        // sub_8246C5D8: every second node from the candidate node on.
        let mut best = f32::MAX;
        let mut i = node;
        while i < count {
            let p = line.nodes[i as usize].position;
            for r in ctx.players {
                let d = sub(p, *r);
                best = best.min(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
            }
            i += 2;
        }
        let near = if best == f32::MAX { retail::BRANCH_NEAR_CAP } else { ((best.sqrt() * retail::BRANCH_NEAR_SCALE) as i64).min(retail::BRANCH_NEAR_CAP) };
        score += near;
    }
    if line.flags & 7 == 7 {
        score += retail::BRANCH_ALL_TYPES;
    }
    if ctx.preferred_skill != -1 && line.skill != -1 && line.skill != ctx.preferred_skill {
        score += i64::from((line.skill - ctx.preferred_skill).abs()) * retail::BRANCH_SKILL_SCALE + retail::BRANCH_SKILL_BASE;
    }
    Some(score)
}

/// The nearest node to `position` among `target - 3 ..= target` (`sub_82455BB0`).
pub fn rejoin_node(line: &ReplayLine, target: u32, position: Vec3) -> u32 {
    let last = line.nodes.len().saturating_sub(1) as u32;
    let start = target.saturating_sub(retail::BRANCH_REJOIN_BACK).min(last);
    let end = target.min(last);
    if end <= start {
        return start;
    }
    let mut best = (start, f32::MAX);
    for i in start..=end {
        let d = sub(line.nodes[i as usize].position, position);
        let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if d2 < best.1 {
            best = (i, d2);
        }
    }
    best.0
}

/// The branch choice at a group node (`sub_8246BEE0`): `Some((line, node))` when a branch target
/// wins, `None` when the stay wins or every candidate is rejected.
pub fn choose_branch(lines: &dyn LineSource, current: &ReplayLine, node: u32, group: &ReplayBranchGroup, ctx: &BranchContext) -> Option<([u8; 16], u32)> {
    let mut best: Option<(usize, i64)> = None;
    let mut candidates: Vec<(&ReplayLine, u32)> = vec![(current, node)];
    for b in &group.branches {
        if b.target == current.id || ctx.in_use.contains(&b.target) || !ctx.valid(&b.target) {
            continue;
        }
        if let Some(l) = lines.line(&b.target) {
            if (b.target_node as usize) < l.nodes.len() {
                candidates.push((l, b.target_node));
            }
        }
    }
    for (i, (l, n)) in candidates.iter().enumerate() {
        if let Some(s) = branch_score(l, *n, ctx) {
            if best.is_none_or(|(_, b)| s < b) {
                best = Some((i, s));
            }
        }
    }
    match best {
        Some((i, _)) if i > 0 => {
            let (l, n) = candidates[i];
            Some((l.id, rejoin_node(l, n, ctx.position)))
        }
        _ => None,
    }
}

/// The next line at the end of `current` (8246D3C0 -> 8246C7F8, candidates 82458968 mode 1): docs/hails-additions/26c-living-world-npc-skaters.md#choose-next-line
pub fn choose_next_line(lines: &dyn LineSource, current: &ReplayLine, ctx: &BranchContext) -> Option<([u8; 16], u32)> {
    let radius = ctx.chain.radius;
    if !(radius > 0.0) || ctx.chain.max_candidates == 0 {
        return None;
    }
    // A simulated skater searches from its body point (`82467FC8`), the replay tier from the node it reached (b701).
    let end = ctx.body.map_or(current.nodes.last()?.position, |b| b.point);
    let r2 = radius * radius;
    let mut candidates: Vec<&ReplayLine> = Vec::new();
    // The fallback's nearest valid and nearest invalid start at any distance (first in order on a tie).
    let mut nearest_valid: Option<(&ReplayLine, f32)> = None;
    let mut nearest_invalid: Option<(&ReplayLine, f32)> = None;
    lines.for_each_line(&mut |l| {
        // No own-line or in-use skip: retail's line test (`82458860`) filters only by the AI's path-kind pattern
        // (`82457620`, every ambient line passes) and the 0x200 object overlap (research b707 / b715 trace).
        if candidates.len() >= ctx.chain.max_candidates {
            return;
        }
        let Some(start) = l.nodes.first() else { return };
        if l.nodes.len() < 2 {
            return;
        }
        let d = sub(start.position, end);
        let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        let valid = ctx.valid(&l.id);
        if valid && d2 <= r2 {
            candidates.push(l);
        } else {
            let nearest = if valid { &mut nearest_valid } else { &mut nearest_invalid };
            if nearest.is_none_or(|(_, n)| d2 < n) {
                *nearest = Some((l, d2));
            }
        }
    });
    match candidates.len() {
        // `82458968` mode 1 with no hit: the nearest invalid start wins when it is no farther than the nearest valid one
        // and that is at least `fallback_valid_d2` away; no valid start -> the nearest invalid (b701 R10).
        0 => ctx.chain.fallback.then(|| match (nearest_valid, nearest_invalid) {
            (Some(v), Some(i)) if i.1 <= v.1 && v.1 >= ctx.chain.fallback_valid_d2 => Some(i),
            (Some(v), _) => Some(v),
            (None, i) => i,
        }).flatten().map(|(l, _)| (l.id, 0)),
        1 => Some((candidates[0].id, 0)),
        _ => {
            let mut best: Option<(usize, i64)> = None;
            for (i, l) in candidates.iter().enumerate() {
                if let Some(s) = branch_score(l, 0, ctx) {
                    if best.is_none_or(|(_, b)| s < b) {
                        best = Some((i, s));
                    }
                }
            }
            Some((candidates[best.map_or(0, |b| b.0)].id, 0))
        }
    }
}

/// Follows one line at the recording rate. Copyable state, no references: a client rebuilds it
/// from the spawn record (line, node 0) and the frame count.
#[derive(Clone, Debug, PartialEq)]
pub struct LineCursor {
    pub line: [u8; 16],
    pub node: u32,
    /// 60 Hz frames into the segment `node -> node + 1`.
    pub frame_in_segment: u32,
    /// 60 Hz frames since spawn.
    pub frames: u64,
    pub finished: bool,
    trick_open: bool,
    /// Recorded trick of the open span (`-1` = none / no slot).
    trick: i16,
    /// Frame `trick` started.
    trick_since: u64,
    /// Entered phases, newest first (`history[0]` = the current phase).
    history: [Option<PhaseEntry>; PHASE_HISTORY],
    phase: Option<ReplayPhase>,
    phase_since: u64,
    /// The phase the current one replaced and the frame it began (puppet crossfade).
    previous_phase: Option<ReplayPhase>,
    previous_since: u64,
    /// The last branch or chain while its root blend may still run (see [`LineSwitch`]).
    pub switch: Option<LineSwitch>,
    /// Root blend time after a switch (host and client set it from [`ChainConfig::blend_seconds`]).
    pub switch_blend_seconds: f32,
    /// Ride the current line turned 180 deg (fix 16, with `keep_facing`); pure function of lines and records.
    pub facing_flipped: bool,
    /// Keep the facing across switches (mod option, not retail; host and client set it from
    /// [`ChainConfig::keep_facing`]).
    pub keep_facing: bool,
    /// Retail latched switch / fakie flip (controller +927), latched at takeoff (8246A700): docs/hails-additions/26c-living-world-npc-skaters.md#latched-flip
    pub flip: bool,
    /// Whether the current node counted as airborne at the last step (the latch's edge detector,
    /// controller `+928`).
    pub airborne: bool,
    /// How the body is drawn (host and client set it from [`ChainConfig::facing_rule`]).
    pub facing_rule: FacingRule,
    /// Retail riding-fakie bit of the drawn body (UpdateRidingFakie82BB2330): docs/hails-additions/26c-living-world-npc-skaters.md#riding-fakie-bit
    pub fakie: bool,
    /// Frame the fakie bit last changed and the frame the change before it happened (the fakie
    /// channel's fade in / out, [`LineCursor::fakie_channel_weight`]).
    pub fakie_since: u64,
    pub fakie_previous_since: u64,
    /// The rule's two clocks (`UpdateRidingFakie` instance state).
    pub fakie_clock: crate::animation::riding_fakie::State,
    /// Thresholds (host and client set them from [`ChainConfig::fakie`]).
    pub fakie_settings: crate::animation::riding_fakie::Settings,
    /// Hold on the last node with nothing to chain to instead of finishing: a physics-simulated skater (retail has no
    /// finished state, b118). Host and client set it from the replicated `follow_target`, so a mirrored cursor holds
    /// where the host holds and applies its chain record when it comes.
    pub hold_at_end: bool,
}

/// How a cursor takes branches: the host decides, a client mirrors records.
pub enum Decider<'a> {
    /// Retail branch choice with this context; decisions are appended to the cursor events.
    Decide(BranchContext<'a>),
    /// Apply recorded decisions (branches matched by frame and from-node, tricks by frame and
    /// node; a slot without a record keeps its recorded trick); never decides.
    Mirror(&'a [BranchRecord], &'a [TrickRecord]),
    /// Never branch (tests, a mod that pins a line).
    Stay,
}

impl LineCursor {
    pub fn new(line: [u8; 16], node: u32) -> Self {
        Self { line, node, frame_in_segment: 0, frames: 0, finished: false, trick_open: false, trick: -1, trick_since: 0, history: [None; PHASE_HISTORY], phase: None, phase_since: 0, previous_phase: None, previous_since: 0, switch: None, switch_blend_seconds: SWITCH_BLEND_SECONDS, facing_flipped: false, keep_facing: false, flip: false, airborne: false, facing_rule: FacingRule::RidingEntry, fakie: false, fakie_since: 0, fakie_previous_since: 0, fakie_clock: Default::default(), fakie_settings: retail::FAKIE, hold_at_end: false }
    }

    /// Spawn on a line at a node (8245DA78; node world frame 8245C548 -> 82453C58): docs/hails-additions/26c-living-world-npc-skaters.md#cursor-spawn
    pub fn spawn(lines: &dyn LineSource, line: [u8; 16], node: u32) -> Self {
        let mut c = Self::new(line, node);
        if let Some(l) = lines.line(&line) {
            c.set_trick(l, node);
            c.phase = l.nodes.get(node as usize).map(|n| ReplayPhase::of(n.flags, c.trick_open));
            c.history[0] = c.phase.map(|phase| PhaseEntry { phase, since: 0, trick: c.trick, trick_since: 0 });
            if let Some(n) = l.nodes.get(node as usize) {
                // In the air: retail's first update sees the rising edge against the spawn frame.
                // Riding: a labelled stand-in (retail holds 0 until the first takeoff, which tests
                // the same frames then); off board: nothing latched.
                c.airborne = node_airborne(n.flags);
                c.flip = (c.airborne || node_riding(n.flags)) && flip_test(n, node_world_frame(n));
            }
        } else {
            c.finished = true;
        }
        c
    }

    /// The simulated skater's line end (`sub_8246D3C0` -> `sub_8246C7F8`, b701): the path follower reported the end
    /// (on the path past t 0.8 of the second-last segment, or off it near the end), so the next line is chosen from
    /// `ctx` (the body point) and the cursor switches from wherever it is, mid-segment included. `None` = no next line
    /// (the skater holds and searches again next tick): docs/hails-additions/26e-living-world-skater-ai.md#line-end-trigger
    pub fn line_end_chain(&mut self, lines: &dyn LineSource, ctx: &BranchContext, out: &mut Vec<CursorEvent>) -> Option<([u8; 16], u32)> {
        let line = lines.line(&self.line)?;
        let (to_line, to_node) = choose_next_line(lines, line, ctx)?;
        let next = lines.line(&to_line)?;
        if to_node as usize + 1 >= next.nodes.len() {
            return None;
        }
        out.push(CursorEvent::Branch(BranchRecord { frame: self.frames, from_line: self.line, from_node: self.node, to_line, to_node, respawn: false }));
        self.apply_switch(line, next, to_line, to_node);
        Some((to_line, to_node))
    }

    /// A host restart on `line` / `node` (re-place, re-snap, bail respawn, walk-back hand back): the spawn state with
    /// the frame clock kept running, so frames never repeat and records stay unique; recorded (a `respawn`
    /// [`BranchRecord`]) so a client restarts its mirrored cursor the same way.
    pub fn respawn(&mut self, lines: &dyn LineSource, line: [u8; 16], node: u32, out: &mut Vec<CursorEvent>) {
        out.push(CursorEvent::Branch(BranchRecord { frame: self.frames, from_line: self.line, from_node: self.node, to_line: line, to_node: node, respawn: true }));
        self.restart(lines, line, node);
    }

    /// [`LineCursor::spawn`] at this cursor's frame (every frame-valued field shifted by it); the tuning fields kept.
    fn restart(&mut self, lines: &dyn LineSource, line: [u8; 16], node: u32) {
        let base = self.frames;
        let (blend, keep_facing, facing_rule, fakie_settings, hold) = (self.switch_blend_seconds, self.keep_facing, self.facing_rule, self.fakie_settings, self.hold_at_end);
        *self = Self::spawn(lines, line, node);
        (self.switch_blend_seconds, self.keep_facing, self.facing_rule, self.fakie_settings, self.hold_at_end) = (blend, keep_facing, facing_rule, fakie_settings, hold);
        self.frames += base;
        self.trick_since += base;
        self.phase_since += base;
        self.previous_since += base;
        self.fakie_since += base;
        self.fakie_previous_since += base;
        for h in self.history.iter_mut().flatten() {
            h.since += base;
            h.trick_since += base;
        }
    }

    /// Switch to `to_line` at `to_node` (a branch, a chain or a mirrored record).
    fn apply_switch(&mut self, line: &ReplayLine, next: &ReplayLine, to_line: [u8; 16], to_node: u32) {
        self.begin_switch(line, next, to_node);
        self.line = to_line;
        self.node = to_node;
        self.frame_in_segment = 0;
        self.set_trick(next, to_node);
        self.finished = false;
    }

    /// A client applies the host's switch or restart recorded at its current frame: a simulated skater's line end
    /// switches mid-segment (`line_end_chain`) and its restarts happen anywhere, which no node arrival would meet. Run
    /// by `step`, and once a tick before stepping (a restart on a tick the host's clock did not move takes no step).
    pub fn apply_records(&mut self, lines: &dyn LineSource, decider: &Decider, out: &mut Vec<CursorEvent>) {
        let Decider::Mirror(records, _) = decider else { return };
        let Some(r) = records.iter().find(|r| r.frame == self.frames && r.from_line == self.line && r.from_node == self.node).cloned() else { return };
        if r.respawn {
            out.push(CursorEvent::Branch(r.clone()));
            self.restart(lines, r.to_line, r.to_node);
            if lines.line(&self.line).is_none() {
                self.finished = true;
                out.push(CursorEvent::Finished);
            }
        } else if let (Some(line), Some(next)) = (lines.line(&self.line), lines.line(&r.to_line).filter(|n| (r.to_node as usize) + 1 < n.nodes.len())) {
            out.push(CursorEvent::Branch(r.clone()));
            self.apply_switch(line, next, r.to_line, r.to_node);
        }
    }

    /// Advance one 60 Hz frame.
    pub fn step(&mut self, lines: &dyn LineSource, decider: &mut Decider, out: &mut Vec<CursorEvent>) {
        if self.finished {
            return;
        }
        let Some(mut line) = lines.line(&self.line) else {
            self.finished = true;
            out.push(CursorEvent::Finished);
            return;
        };
        if matches!(decider, Decider::Mirror(..)) {
            self.apply_records(lines, decider, out);
            if self.finished {
                return;
            }
            line = match lines.line(&self.line) {
                Some(l) => l,
                None => return,
            };
        }
        // The character's forward before this frame (the drawn skater), for the flip latch; only
        // needed while not airborne (a rising edge can follow).
        let before = (!self.airborne).then(|| self.drawn_skater(line, 0.0));
        if self.node as usize + 1 >= line.nodes.len() {
            // On the last node (spawned there, or a mirrored record not yet seen): look for the
            // next line once more, else the line is over.
            match self.chain(lines, line, decider, out) {
                Some(next) => line = next,
                // A simulated skater holds on the last node and searches again next tick (retail has no finished
                // state, b118 / b701); the replay tier ends its line.
                None if self.hold_at_end || matches!(decider, Decider::Decide(ctx) if ctx.body.is_some()) => return,
                None => {
                    self.finished = true;
                    out.push(CursorEvent::Finished);
                    return;
                }
            }
        }
        self.frames += 1;
        self.frame_in_segment += 1;
        // Bounded: zero-frame segments are crossed at once, a branch may land on one.
        for _ in 0..256 {
            if self.node as usize + 1 >= line.nodes.len() {
                break;
            }
            let seg = line.segment_frames(self.node);
            if self.frame_in_segment < seg {
                break;
            }
            self.frame_in_segment -= seg;
            self.node += 1;
            let n = &line.nodes[self.node as usize];
            match n.event {
                node_events::START_TRICK => {
                    let recorded = line.node_trick(n);
                    let open = self.trick_open && self.trick >= 0;
                    let chosen = match decider {
                        Decider::Decide(ctx) => match super::npc_tricks::choose(line, self.node, self.frames, &ctx.tricks) {
                            super::npc_tricks::TrickChoice::Start(t) => Some(t),
                            super::npc_tricks::TrickChoice::Continue if open => None,
                            super::npc_tricks::TrickChoice::Continue => Some(recorded),
                        },
                        Decider::Mirror(_, tricks) => match tricks.iter().find(|r| r.frame == self.frames && r.line == self.line && r.node == self.node) {
                            Some(r) => Some(r.chosen),
                            None if open && crate::scoring::catalog::is_chain_level(recorded) => None,
                            None => Some(recorded),
                        },
                        Decider::Stay => Some(recorded),
                    };
                    // A recorded trick the choice dropped (-1) starts nothing (8246A2E0; b187, b159).
                    if recorded < 0 || chosen.is_none_or(|t| t >= 0) {
                        self.trick_open = true;
                    }
                    if let Some(t) = chosen {
                        self.trick = t;
                        self.trick_since = self.frames;
                        if matches!(decider, Decider::Decide(_)) {
                            out.push(CursorEvent::Trick(TrickRecord { frame: self.frames, line: self.line, node: self.node, recorded, chosen: t }));
                        }
                    }
                }
                node_events::END_TRICK => {
                    self.trick_open = false;
                    self.trick = -1;
                }
                _ => {}
            }
            out.push(CursorEvent::Node { line: self.line, node: self.node, event: n.event, flags: n.flags });
            if let Some(group) = line.group_at(self.node) {
                let choice = match decider {
                    Decider::Decide(ctx) => match ctx.body {
                        // A simulated skater: gated, and scored / rejoined from its body point (b674).
                        Some(body) if !body.may_branch => None,
                        Some(body) => {
                            let mut here = *ctx;
                            here.position = body.point;
                            choose_branch(lines, line, self.node, group, &here)
                        }
                        // The replay skater stands on the node, moving along the segment it just rode.
                        None => {
                            let mut here = *ctx;
                            here.position = line.nodes[self.node as usize].position;
                            let v = sub(here.position, line.nodes[self.node as usize - 1].position);
                            if v[0].hypot(v[2]) > 1e-4 {
                                here.forward = v;
                            }
                            choose_branch(lines, line, self.node, group, &here)
                        }
                    },
                    Decider::Mirror(records, _) => records.iter().find(|r| !r.respawn && r.frame == self.frames && r.from_line == self.line && r.from_node == self.node).map(|r| (r.to_line, r.to_node)),
                    Decider::Stay => None,
                };
                if let Some((to_line, to_node)) = choice {
                    if let Some(next) = lines.line(&to_line) {
                        out.push(CursorEvent::Branch(BranchRecord { frame: self.frames, from_line: self.line, from_node: self.node, to_line, to_node, respawn: false }));
                        self.apply_switch(line, next, to_line, to_node);
                        line = next;
                    }
                }
            }
            // Reached the last node: retail looks for the next line right away (`sub_8246D3C0`).
            if self.node as usize + 1 >= line.nodes.len() {
                if let Some(next) = self.chain(lines, line, decider, out) {
                    line = next;
                }
            }
        }
        if let Some(n) = line.nodes.get(self.node as usize) {
            // Retail flip latch (`sub_8246A700`): only at takeoff, against the character's
            // forward; held otherwise (landings and the switches above included, b354).
            let airborne = node_airborne(n.flags);
            if airborne && !self.airborne {
                if let Some(q) = before {
                    self.flip = flip_test(n, q);
                }
            }
            self.airborne = airborne;
            let phase = ReplayPhase::of(n.flags, self.trick_open);
            if self.phase != Some(phase) {
                if let Some(old) = self.phase {
                    self.previous_phase = Some(old);
                    self.previous_since = self.phase_since;
                }
                self.phase = Some(phase);
                self.phase_since = self.frames;
                self.history.copy_within(0..PHASE_HISTORY - 1, 1);
                self.history[0] = Some(PhaseEntry { phase, since: self.frames, trick: if self.trick_open { self.trick } else { -1 }, trick_since: self.trick_since });
            }
        }
        // A finished switch blend is dropped (the weight only grows from here).
        if self.switch.as_ref().is_some_and(|sw| self.switch_weight(sw, self.frames as f64) >= 1.0) {
            self.switch = None;
        }
        self.update_fakie(line);
    }

    /// One step of the riding-fakie rule on the body as drawn (category mapping NOT RETAIL YET).
    fn update_fakie(&mut self, line: &ReplayLine) {
        let Some(n) = line.nodes.get(self.node as usize) else { return };
        let v = segment_velocity(line, self.node);
        let f = rotate(self.drawn_skater(line, 0.0), [0.0, 0.0, 1.0]);
        let category = if node_riding(n.flags) { 1 } else { 3 };
        let physical = crate::animation::riding_fakie::Physical {
            category,
            grind_state: 0,
            doing_trick: self.trick_open,
            board_axis: [f[0], f[1], f[2], 0.0],
            deck_velocity: [v[0], v[1], v[2], 0.0],
            external_velocity: [v[0], v[1], v[2], 0.0],
            ground_projected_speed: v[0].hypot(v[2]),
        };
        if let Some(fakie) = self.fakie_clock.update(physical, (1.0 / RECORDING_HZ) as f32, self.fakie_settings) {
            if fakie != self.fakie {
                self.fakie = fakie;
                self.fakie_previous_since = self.fakie_since;
                self.fakie_since = self.frames;
            }
        }
    }

    /// Weight of the fakie channel (`B_FAKIE_CHANNEL`, `FakieHeadChannel82BAC778`) `alpha` of the
    /// next frame: fades in over 0.3 s from the frame the bit set, out over 0.3 s from the frame it
    /// cleared (linear, `ChannelPlayback`; the fade out starts from the weight reached).
    pub fn fakie_channel_weight(&self, alpha: f32) -> f32 {
        let fade = retail::FAKIE_CHANNEL_FADE_SECONDS;
        let since = (self.frames - self.fakie_since.min(self.frames)) as f32 + alpha.clamp(0.0, 1.0);
        let t = since / RECORDING_HZ as f32;
        if self.fakie {
            (t / fade).clamp(0.0, 1.0)
        } else if self.fakie_since > 0 {
            let held = (self.fakie_since - self.fakie_previous_since.min(self.fakie_since)) as f32 / RECORDING_HZ as f32;
            (held / fade).clamp(0.0, 1.0).min((1.0 - t / fade).clamp(0.0, 1.0))
        } else {
            0.0
        }
    }

    /// At the last node of `line`: continue on the next line (host: [`choose_next_line`]; client:
    /// the host's record, a [`CursorEvent::Branch`] from the last node). `None` = no next line.
    fn chain<'a>(&mut self, lines: &'a dyn LineSource, line: &'a ReplayLine, decider: &mut Decider, out: &mut Vec<CursorEvent>) -> Option<&'a ReplayLine> {
        let (to_line, to_node) = match decider {
            Decider::Decide(ctx) => {
                let mut here = *ctx;
                here.position = line.nodes.get(self.node as usize)?.position;
                if self.node > 0 {
                    let v = sub(here.position, line.nodes[self.node as usize - 1].position);
                    if v[0].hypot(v[2]) > 1e-4 {
                        here.forward = v;
                    }
                }
                choose_next_line(lines, line, &here)?
            }
            Decider::Mirror(records, _) => records.iter().find(|r| !r.respawn && r.frame == self.frames && r.from_line == self.line && r.from_node == self.node).map(|r| (r.to_line, r.to_node))?,
            Decider::Stay => return None,
        };
        let next = lines.line(&to_line)?;
        if to_node as usize + 1 >= next.nodes.len() {
            return None;
        }
        out.push(CursorEvent::Branch(BranchRecord { frame: self.frames, from_line: self.line, from_node: self.node, to_line, to_node, respawn: false }));
        self.apply_switch(line, next, to_line, to_node);
        Some(next)
    }

    fn set_trick(&mut self, line: &ReplayLine, node: u32) {
        let open = line.open_trick_at(node);
        self.trick_open = open.is_some();
        if open.unwrap_or(-1) != self.trick {
            self.trick_since = self.frames;
        }
        self.trick = open.unwrap_or(-1);
    }

    /// Trick of the open trick span: recorded, or the one the trick choice started (-1 outside a span): docs/hails-additions/26c-living-world-npc-skaters.md#current-trick-and-phases
    pub fn current_trick(&self) -> i16 {
        if self.trick_open {
            self.trick
        } else {
            -1
        }
    }

    pub fn phase_history(&self) -> impl Iterator<Item = (PhaseEntry, u64)> + '_ {
        self.history.iter().flatten().map(|e| (*e, self.frames - e.since.min(self.frames)))
    }

    /// Weight (0..1) of the new line in the drawn root `frames` (fractional) after the switch.
    fn switch_weight(&self, sw: &LineSwitch, frames: f64) -> f32 {
        let elapsed = ((frames - sw.frame as f64) / RECORDING_HZ).max(0.0) as f32;
        crate::animation::playback_transition::transition_weight(elapsed, self.switch_blend_seconds)
    }

    /// The drawn root for the raw line pose at `frames`: the running switch blend applied.
    fn drawn(&self, position: Vec3, skater: [f32; 4], board: [f32; 4], frames: f64) -> (Vec3, [f32; 4], [f32; 4]) {
        match &self.switch {
            Some(sw) => {
                let w = self.switch_weight(sw, frames);
                if w >= 1.0 {
                    return (position, skater, board);
                }
                (std::array::from_fn(|k| position[k] + sw.offset[k] * (1.0 - w)), nlerp(sw.skater, skater, w), nlerp(sw.board, board, w))
            }
            None => (position, skater, board),
        }
    }

    /// At a branch or chain: keep where the skater is drawn so the root blends onto the new line: docs/hails-additions/26c-living-world-npc-skaters.md#begin-switch
    fn begin_switch(&mut self, old: &ReplayLine, next: &ReplayLine, to_node: u32) {
        let (Some(a), Some(b)) = (old.nodes.get(self.node as usize), next.nodes.get(to_node as usize)) else { return };
        let i = self.node as usize;
        let (p, skater, board) = self.drawn(a.position, self.facing(self.rule_skater(old, i, i, 0.0)), self.facing(decode_orientation(a.board)), self.frames as f64);
        if self.keep_facing {
            let f = rotate(skater, [0.0, 0.0, 1.0]);
            let j = to_node as usize;
            let g = rotate(self.facing(self.rule_skater(next, j, j, 0.0)), [0.0, 0.0, 1.0]);
            if f[0].hypot(f[2]) > 1e-3 && g[0].hypot(g[2]) > 1e-3 && yaw_angle(f, g).abs() > std::f32::consts::FRAC_PI_2 {
                self.facing_flipped = !self.facing_flipped;
            }
        } else {
            self.facing_flipped = false;
        }
        self.switch = Some(LineSwitch { frame: self.frames, offset: sub(p, b.position), skater, board });
    }

    /// A recorded orientation as this skater rides it: turned 180 deg about its own up axis
    /// (`q * (0, 1, 0, 0)`) while [`LineCursor::facing_flipped`].
    pub fn facing(&self, q: [f32; 4]) -> [f32; 4] {
        if self.facing_flipped { turn_about_up(q) } else { q }
    }

    /// The skater frame the facing rule gives between nodes `i` and `j` of `line` at fraction
    /// `t` (before [`LineCursor::facing`] and the switch blend).
    pub fn rule_skater(&self, line: &ReplayLine, i: usize, j: usize, t: f32) -> [f32; 4] {
        match self.facing_rule {
            FacingRule::RidingEntry => {
                // 8246ACF0 applies the flip only while airborne (+438, b354); off-board nodes show the recorded frame: docs/hails-additions/26c-living-world-npc-skaters.md#flip-only-in-the-air
                let off = |k: usize| line.nodes.get(k).is_some_and(|n| n.flags & node_flags::OFF_BOARD != 0);
                if !self.flip {
                    target_skater(line, i, j, t)
                } else if !off(i) && !off(j) {
                    turn_about_up(target_skater(line, i, j, t))
                } else {
                    // Stepping on or off the board: slerp between the per-node targets.
                    let node = |k: usize| {
                        let q = line.nodes.get(k).map_or([0.0, 0.0, 0.0, 1.0], |n| decode_orientation(n.skater));
                        if off(k) { q } else { turn_about_up(q) }
                    };
                    slerp(node(i), node(j), t)
                }
            }
            FacingRule::PerNode => nlerp(drawn_skater(line, i), drawn_skater(line, j), t),
        }
    }

    /// Segment nodes and fraction at `alpha` of the next frame: `(i, j, t)`.
    fn segment_at(&self, line: &ReplayLine, alpha: f32) -> (usize, usize, f32) {
        let i = self.node as usize;
        let seg = if !self.finished && i + 1 < line.nodes.len() { line.segment_frames(self.node) } else { 0 };
        let t = if seg > 0 { ((self.frame_in_segment as f32 + alpha.clamp(0.0, 1.0)) / seg as f32).min(1.0) } else { 0.0 };
        (i, if seg > 0 { i + 1 } else { i }, t)
    }

    /// The drawn skater orientation now (`alpha` of the next frame): the facing rule, the fix 16
    /// turn and the running switch blend, as [`LineCursor::sample`] draws it.
    pub fn drawn_skater(&self, line: &ReplayLine, alpha: f32) -> [f32; 4] {
        let (i, j, t) = self.segment_at(line, alpha);
        let a = if self.finished { 0.0 } else { alpha.clamp(0.0, 1.0) };
        self.drawn([0.0; 3], self.facing(self.rule_skater(line, i, j, t)), [0.0, 0.0, 0.0, 1.0], self.frames as f64 + f64::from(a)).1
    }

    /// Advance `frames` 60 Hz frames.
    pub fn advance(&mut self, frames: u32, lines: &dyn LineSource, decider: &mut Decider, out: &mut Vec<CursorEvent>) {
        for _ in 0..frames {
            self.step(lines, decider, out);
        }
    }

    /// The raw recorded pose the AI steers to (path frame, position, per-frame displacement); no drawing rules.
    pub fn line_target(&self, lines: &dyn LineSource) -> Option<super::ai_record::LineTarget> {
        let line = lines.line(&self.line)?;
        let i = self.node as usize;
        let a = line.nodes.get(i)?;
        let seg = line.segment_frames(self.node);
        match line.nodes.get(i + 1) {
            Some(_) if seg > 0 && !self.finished => line_target_at(line, (self.node, self.frame_in_segment as f32 / seg as f32)),
            _ => Some(segment_target(a, a, 0.0)),
        }
    }

    pub fn sample(&self, lines: &dyn LineSource, alpha: f32) -> Option<ReplaySample> {
        let line = lines.line(&self.line)?;
        let i = self.node as usize;
        let a = line.nodes.get(i)?;
        let (b, seg) = match line.nodes.get(i + 1) {
            Some(b) if !self.finished => (b, line.segment_frames(self.node)),
            _ => (a, 0),
        };
        let t = if seg > 0 { ((self.frame_in_segment as f32 + alpha.clamp(0.0, 1.0)) / seg as f32).min(1.0) } else { 0.0 };
        let position = std::array::from_fn(|k| a.position[k] + (b.position[k] - a.position[k]) * t);
        let velocity = segment_velocity(line, self.node);
        let alpha = if self.finished { 0.0 } else { alpha.clamp(0.0, 1.0) };
        let j = if seg > 0 { i + 1 } else { i };
        let skater = self.facing(self.rule_skater(line, i, j, t));
        let board = self.facing(nlerp(decode_orientation(a.board), decode_orientation(b.board), t));
        let (position, skater, board) = self.drawn(position, skater, board, self.frames as f64 + f64::from(alpha));
        let heading = if velocity[0].hypot(velocity[2]) > 0.05 {
            velocity[0].atan2(velocity[2])
        } else {
            let f = rotate(skater, [0.0, 0.0, 1.0]);
            f[0].atan2(f[2])
        };
        let phase = self.phase.unwrap_or_else(|| ReplayPhase::of(a.flags, self.trick_open));
        Some(ReplaySample {
            line: self.line,
            node: self.node,
            position,
            velocity,
            heading,
            board,
            skater,
            flags: a.flags,
            phase,
            jump: a.jump,
            phase_frames: self.frames - self.phase_since.min(self.frames),
            previous_phase: self.previous_phase,
            previous_phase_frames: self.frames - self.previous_since.min(self.frames),
            sub_frame: alpha,
            fakie: self.fakie,
        })
    }

    /// Drawn state `frames_ahead` frames on, stepped with the host's records (never guesses a branch).
    pub fn render_sample(&self, lines: &dyn LineSource, records: &[BranchRecord], tricks: &[TrickRecord], frames_ahead: f32) -> Option<ReplaySample> {
        let (c, frac) = self.render_cursor(lines, records, tricks, frames_ahead);
        c.sample(lines, frac)
    }

    /// The cursor [`LineCursor::render_sample`] samples (whole frames stepped with the records)
    /// and the remaining fraction; its [`LineCursor::phase_history`] drives the puppet's clips.
    pub fn render_cursor(&self, lines: &dyn LineSource, records: &[BranchRecord], tricks: &[TrickRecord], frames_ahead: f32) -> (LineCursor, f32) {
        let ahead = if frames_ahead.is_finite() { frames_ahead.max(0.0) } else { 0.0 };
        let whole = ahead.floor();
        let mut c = self.clone();
        if whole > 0.0 {
            c.advance(whole as u32, lines, &mut Decider::Mirror(records, tricks), &mut Vec::new());
        }
        (c, ahead - whole)
    }
}

/// Velocity of the segment from `node` (or the last one before it when it has no duration), m/s.
pub fn segment_velocity(line: &ReplayLine, node: u32) -> Vec3 {
    let mut i = node as usize;
    loop {
        if i + 1 < line.nodes.len() {
            let f = line.nodes[i + 1].frames;
            if f > 0 {
                let d = sub(line.nodes[i + 1].position, line.nodes[i].position);
                let s = RECORDING_HZ as f32 / f32::from(f);
                return d.map(|c| c * s);
            }
        }
        if i == 0 {
            return [0.0; 3];
        }
        i -= 1;
    }
}

/// Nearest point of `line` to `p` from `from` on (82462C18 projection; search window not decoded).
pub fn project_on_line(line: &ReplayLine, from: u32, p: Vec3, max_distance: f32) -> Option<super::avoid::PathPoint> {
    let n = line.nodes.len();
    if n == 0 {
        return None;
    }
    let mut best: Option<(f32, super::avoid::PathPoint)> = None;
    let mut walked = 0.0f32;
    let mut i = (from as usize).min(n - 1);
    loop {
        let a = &line.nodes[i];
        let (b, j) = if i + 1 < n { (&line.nodes[i + 1], i + 1) } else { (a, i) };
        let ab = sub(b.position, a.position);
        let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        let ap = sub(p, a.position);
        let t = if l2 > 1e-8 { ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let q: Vec3 = core::array::from_fn(|k| a.position[k] + ab[k] * t);
        let d = sub(p, q);
        let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if best.as_ref().is_none_or(|b| d2 < b.0) {
            // Segment direction; a zero-length segment falls back to the line's recorded motion.
            let raw = if l2 > 1e-8 { ab } else { segment_velocity(line, i as u32) };
            let rl = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2]).sqrt();
            let direction = if rl > 1e-6 { raw.map(|c| c / rl) } else { [0.0, 0.0, 1.0] };
            let w = |k: usize| (f32::from(a.width[k]) + (f32::from(b.width[k]) - f32::from(a.width[k])) * t) * WIDTH_SCALE;
            best = Some((d2, super::avoid::PathPoint { point: q, direction, width_left: w(0), width_right: w(1), node: i as u32, t }));
        }
        walked += l2.sqrt();
        if j == i || walked > max_distance {
            break;
        }
        i = j;
    }
    best.map(|b| b.1)
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
