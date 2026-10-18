//! Controller B (retail "NavMeshController"), the NPC skater's walk back to its line for the obstacle avoider's
//! mode 7 ("back to the line past the prop"; retail TU3, evidence only; re-implemented; `.local/research/npc/b70-npc-submode-reposition.md` and
//! `b78-mode7-controller-b.md`, `b79-controller-b-outputs-path.md`; main checked the constants, the throttle `82467B00`,
//! the reset `82466F40` and the writer slots `82470D08` / `82470D78`):
//! - the brain swaps from the line controller (A) to B (`sub_824661A8`); B's activate (`82467508`) resets it
//!   (`82466F40`: everything 0, waypoint seen -1, the "can move" latch = the skater's state now) and plans a path
//!   to the chosen line node (`82467088`): with waypoints the last one must lie within 2.0 h / 1.0 v of the node,
//!   without waypoints the node within 1.5 h / 3.0 v of the skater; a failed plan keeps A;
//! - B's tick (`824675C8`) writes the on-foot intents, never position or velocity: abort / hand back, the
//!   `WipeOutRequest` (key `0x830BECC4`) on a stop request or when stuck, then the path update (`82467718`: latch,
//!   re-plan, arrival, waypoint advance, stuck monitor `82466A00`), the off-board toggle presses (`824712E0`,
//!   "ToggleOffboardState": `NewToggleOffBoardState` / `ToggleOffBoardState` until the skater is off the board),
//!   the steer (`82467C90`, "OffBoardMotion" slot 2 `82470D78`: `OB_Steer` and `OB_MagHeld` = s) and the throttle
//!   (`82467B00`, slot 1 `82470D08`: `OB_Mag` and `OB_MagHeld` = min(f, 1), only when f > 0); so the skater steps
//!   off and walks back on the navmesh (NavPower in retail, the peds' navmesh here);
//! - B hands back to A (`sub_824661A8(brain, null)`: A re-attaches by its saved line id) on arrival (all legs done,
//!   2.4 h / 1.0 v of the node, heading within pi/8 of the node direction), when no plan is left, or on abort.
//!
//! Distances are horizontal (x, z) and vertical (y). Angles are signed about +Y. Multiplayer: [`ControllerB`] is
//! plain per-NPC data, ticked by the host at the retail tick; [`PathService`] must be deterministic.

use std::f32::consts::PI;

/// Retail numbers (image constants); every field is data a mod can override.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerBSettings {
    /// Plan acceptance with waypoints: the last waypoint to the node (`0x82257218` 2.0 h, `0x8225721C` 1.0 v).
    pub plan_path_h: f32,
    pub plan_path_v: f32,
    /// Plan acceptance without waypoints: the skater to the node (`0x82257224` 1.5 h, `0x82257220` 3.0 v).
    pub plan_direct_h: f32,
    pub plan_direct_v: f32,
    /// Arrival (`82467E48`): `0x820993F0` 2.4 h, `0x8231A844` 1.0 v, heading `0x822F8B48` pi/8.
    pub arrive_h: f32,
    pub arrive_v: f32,
    pub arrive_heading: f32,
    /// Line-up zone (`82467C90`): steer along the node direction within `0x822570E0` 2.2 h / `0x8208824C` 0.8 v.
    pub line_up_h: f32,
    pub line_up_v: f32,
    /// "On a leg" (`82467968`): `0x82257228` 0.5 h, `0x8225722C` 2.0 v; degenerate below `0x8209BE90` 1e-4.
    pub leg_h: f32,
    pub leg_v: f32,
    pub leg_epsilon_sq: f32,
    /// Steer: full lock from `0x821DBCF0` pi/6, linear gain `0x822F9288` 6/pi below it.
    pub steer_full_lock: f32,
    pub steer_gain: f32,
    /// Throttle: `0x8209975C` 0.5 x distance^2; below `0x820C6D98` 0.25 along it scales by along x `0x82257308`
    /// 4.0; floor `0x820D06C0` 0.3 when along > `0x822F9128` 0.7071; cut when not moved for more than 10 ticks
    /// with |last steer| above 0.25.
    pub throttle_gain: f32,
    pub slow_along: f32,
    pub slow_gain: f32,
    pub throttle_floor: f32,
    pub floor_along: f32,
    pub stall_ticks: u32,
    pub stall_steer: f32,
    /// Off-board toggle presses (`824712E0`): one press pair once the count passes this (4, so every 5 ticks).
    pub push_period: u32,
    /// `OB_Mag` cap of the OffBoardMotion writer (`82470D08`, `0x8231A844` 1.0).
    pub magnitude_cap: f32,
    /// Stuck monitor (`82466A00`): 600 ticks on one waypoint; 90 stalled ticks, cleared after 45 recovery ticks;
    /// moved = step^2 >= `0x822F935C` 0.0009 or from the anchor >= `0x820C6D98` 0.25; turned = more than
    /// `0x822F91AC` 5 deg per tick or `0x822F9360` 10 deg from the anchor.
    pub stuck_waypoint_ticks: u32,
    pub stuck_stall_ticks: u32,
    pub stall_recovery_ticks: u32,
    pub move_step_sq: f32,
    pub move_anchor_sq: f32,
    pub turn_step: f32,
    pub turn_anchor: f32,
}

impl Default for ControllerBSettings {
    fn default() -> Self {
        Self {
            plan_path_h: 2.0,
            plan_path_v: 1.0,
            plan_direct_h: 1.5,
            plan_direct_v: 3.0,
            arrive_h: 2.4,
            arrive_v: 1.0,
            arrive_heading: PI / 8.0,
            line_up_h: 2.2,
            line_up_v: 0.8,
            leg_h: 0.5,
            leg_v: 2.0,
            leg_epsilon_sq: 1e-4,
            steer_full_lock: PI / 6.0,
            steer_gain: 6.0 / PI,
            throttle_gain: 0.5,
            slow_along: 0.25,
            slow_gain: 4.0,
            throttle_floor: 0.3,
            floor_along: 0.7071,
            stall_ticks: 10,
            stall_steer: 0.25,
            push_period: 4,
            magnitude_cap: 1.0,
            stuck_waypoint_ticks: 600,
            stuck_stall_ticks: 90,
            stall_recovery_ticks: 45,
            move_step_sq: 0.0009,
            move_anchor_sq: 0.25,
            turn_step: 5f32.to_radians(),
            turn_anchor: 10f32.to_radians(),
        }
    }
}

/// One element of a solved path (`82466860` / `82466930`): its leg runs from `entry` to `exit` [inference: portal
/// entry / exit]; the aim point while on it is `entry`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathElement {
    pub entry: [f32; 3],
    pub exit: [f32; 3],
}

/// The path service behind the plan (`829277C0`, path object `828AAE90`).
pub trait PathService {
    /// A path from `from` to `to`: `None` = no path, an empty list = go straight.
    fn solve(&self, from: [f32; 3], to: [f32; 3]) -> Option<Vec<PathElement>>;
    /// The per-tick re-solve of a held path; false forces a re-plan.
    fn still_valid(&self, _path: &[PathElement], _from: [f32; 3]) -> bool {
        true
    }
}

/// No navigation: always the retail "no waypoints" branch (exact for targets within the direct acceptance). With no
/// waypoints and the heading `B+112` still zero after the reset (angle 0), B hands back on its first tick whenever the
/// node is within the arrival distance, as retail does.
#[derive(Clone, Copy, Debug, Default)]
pub struct DirectPath;

impl PathService for DirectPath {
    fn solve(&self, _from: [f32; 3], _to: [f32; 3]) -> Option<Vec<PathElement>> {
        Some(Vec::new())
    }
}

/// The peds' navmesh as B's path service (retail: NavPower, the same runtime the peds use; b79). Every path corner,
/// the goal included, becomes one element with entry = exit = the corner [inference: retail's 56-byte elements carry
/// two points per portal; ours are the funnel's corners], so a reachable goal always has a last waypoint for the
/// 2.0 h / 1.0 v acceptance. The mesh is static, so a held path stays valid (retail re-checks its element references
/// each tick, `829277C0`).
impl PathService for super::peds::NavMesh {
    fn solve(&self, from: [f32; 3], to: [f32; 3]) -> Option<Vec<PathElement>> {
        let (a, b) = (self.locate(from)?, self.locate(to)?);
        let corners = self.find_path(a, b)?;
        Some(corners.into_iter().map(|c| PathElement { entry: c, exit: c }).collect())
    }
}

/// `sub_82466698`: x^2 + z^2 < h^2 and |y| < v.
fn within(d: [f32; 3], h: f32, v: f32) -> bool {
    d[0] * d[0] + d[2] * d[2] < h * h && d[1].abs() < v
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn flat(v: [f32; 3]) -> [f32; 3] {
    [v[0], 0.0, v[2]]
}

/// `8296EC98(a, b, up)` with the caller's wrap: the angle between `a` and `b` (flattened), positive when
/// (a x b) . up > 0, in -pi..pi; 0 when either vector's length^2 is at most 1e-4 (`0x8209BE90`).
fn signed_angle(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (a, b) = (flat(a), flat(b));
    if dot(a, a) <= 1e-4 || dot(b, b) <= 1e-4 {
        return 0.0;
    }
    let cross_y = a[2] * b[0] - a[0] * b[2];
    cross_y.atan2(dot(a, b))
}

fn wrap(a: f32) -> f32 {
    let mut a = a % (2.0 * PI);
    if a > PI {
        a -= 2.0 * PI;
    } else if a < -PI {
        a += 2.0 * PI;
    }
    a
}

/// The stuck monitor at `B+128` (`82466A00`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuckMonitor {
    pub last_position: [f32; 3],
    pub anchor_position: [f32; 3],
    pub last_facing: f32,
    pub anchor_facing: f32,
    /// `192` (the waypoint index seen, -1 after a reset) and `196` (ticks on it).
    pub waypoint_seen: i64,
    pub waypoint_ticks: u32,
    /// `200` stalled ticks, `204` recovery ticks, `208` ticks not moved, `212` ticks not turned.
    pub stall: u32,
    pub recovery: u32,
    pub no_move: u32,
    pub no_turn: u32,
    /// `232`.
    pub stuck: bool,
}

impl Default for StuckMonitor {
    fn default() -> Self {
        Self { last_position: [0.0; 3], anchor_position: [0.0; 3], last_facing: 0.0, anchor_facing: 0.0, waypoint_seen: -1, waypoint_ticks: 0, stall: 0, recovery: 0, no_move: 0, no_turn: 0, stuck: false }
    }
}

impl StuckMonitor {
    fn tick(&mut self, s: &ControllerBSettings, position: [f32; 3], facing: f32, waypoint: usize, can_move: bool) {
        if self.waypoint_seen != waypoint as i64 {
            self.waypoint_seen = waypoint as i64;
            self.waypoint_ticks = 0;
        } else {
            self.waypoint_ticks += 1;
            if self.waypoint_ticks > s.stuck_waypoint_ticks {
                self.stuck = true;
            }
        }
        let step = sub(position, self.last_position);
        let from_anchor = sub(position, self.anchor_position);
        let moved = dot(step, step) >= s.move_step_sq || dot(from_anchor, from_anchor) >= s.move_anchor_sq;
        let turned = wrap(facing - self.last_facing).abs() > s.turn_step || wrap(facing - self.anchor_facing).abs() > s.turn_anchor;
        self.no_move = if moved { 0 } else { self.no_move + 1 };
        self.no_turn = if turned { 0 } else { self.no_turn + 1 };
        if !moved && !turned && can_move {
            self.stall += 1;
            self.recovery = 0;
        } else if self.stall > 0 {
            self.recovery += 1;
            if self.recovery > s.stall_recovery_ticks {
                self.stall = 0;
                self.recovery = 0;
            }
        }
        if self.stall > s.stuck_stall_ticks {
            self.stuck = true;
        }
        self.last_position = position;
        self.last_facing = facing;
        if moved {
            self.anchor_position = position;
        }
        if turned {
            self.anchor_facing = facing;
        }
    }
}

/// What the skater reports to B each tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BInput {
    /// The skater position (`[rec+20]+416`).
    pub position: [f32; 3],
    /// `B+112` = `[rec+20]+0` ([inference] the velocity or forward vector).
    pub body: [f32; 3],
    /// The facing angle the stuck monitor compares (`82592A78(S)+32`), radians about +Y.
    pub facing: f32,
    /// Skater flag `+1904` 0x20000000: abort.
    pub abort_flag: bool,
    /// State byte `[rec+28]+59`: hand back.
    pub state_hand_back: bool,
    /// Skater flag `+1904` 0x04000000 ("online"): B does nothing.
    pub online: bool,
    /// `82466ED0`: `[rec+56]+161` set, `+160`, `[rec+72]+308`, `+309` clear ([inference] off the board, standing).
    pub can_move: bool,
    /// State bytes `[rec+56]+161` = SkaterOffBoard (Processed +2484 0x08000000) / `+160` = TransitioningOnOffBoard
    /// (+2480 0x4), written by `82DB6EC0` (b80).
    pub byte_161: bool,
    pub byte_160: bool,
}

/// The controls of one tick (the control map `out`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BControls {
    /// `OB_Steer` (OffBoardMotion slot 2), every tick: -1..1.
    pub steer: f32,
    /// `OB_Mag` (slot 1, capped at `magnitude_cap`), only when above 0.
    pub throttle: Option<f32>,
    /// `OB_MagHeld`: written by the steer, then by the throttle when it is sent.
    pub mag_held: f32,
    /// `WipeOutRequest` (key `0x830BECC4`) = 1.0: a stop request or stuck.
    pub wipe_out: bool,
    /// The off-board toggle press pair (`NewToggleOffBoardState` `0x830BEACC`, `ToggleOffBoardState` `0x830C0350`).
    pub push: bool,
}

impl BControls {
    /// The control-map entries (intent name, value) in retail's write order; names from the key globals'
    /// static initialisers (b79).
    pub fn intents(&self) -> Vec<(&'static str, f32)> {
        let mut v = Vec::new();
        if self.wipe_out {
            v.push(("WipeOutRequest", 1.0));
        }
        if self.push {
            v.push(("NewToggleOffBoardState", 1.0));
            v.push(("ToggleOffBoardState", 1.0));
        }
        v.push(("OB_Steer", self.steer));
        if let Some(f) = self.throttle {
            v.push(("OB_Mag", f));
        }
        v.push(("OB_MagHeld", self.mag_held));
        v
    }
}

/// Why B hands back to A.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandBack {
    Arrived,
    NoPlan,
    Aborted,
}

/// One tick's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BTick {
    /// Online: nothing written.
    Idle,
    Controls(BControls),
    /// Hand back to A; `wipe_out` was already written this tick.
    HandBack { reason: HandBack, wipe_out: bool },
}

/// Controller B's state (`B+28..+259`; the path object `B+240`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControllerB {
    /// `B+80` / `B+96`: the line node and its direction.
    pub target: [f32; 3],
    pub target_dir: [f32; 3],
    pub path: Vec<PathElement>,
    /// `252`.
    pub waypoint: usize,
    /// `256`, `257`, `258`, `259`.
    pub plan_valid: bool,
    pub abort: bool,
    pub stop_request: bool,
    pub can_move: bool,
    /// `B+32` / `B+48` (y = 0).
    pub aim: [f32; 3],
    pub aim_vector: [f32; 3],
    /// `B+112`.
    pub body: [f32; 3],
    /// `244` / `248`.
    pub heading_error: f32,
    pub last_steer: f32,
    pub stuck: StuckMonitor,
    /// The push-state object `M` (`M+0` mode, `M+4` tick count).
    pub push_mode: u8,
    pub push_ticks: u32,
}

impl ControllerB {
    /// B.activate (`82467508`, `824672C8`): reset, then plan to `target` / `target_dir`. `None` = the plan failed
    /// and A stays active.
    pub fn activate(s: &ControllerBSettings, paths: &dyn PathService, target: [f32; 3], target_dir: [f32; 3], input: &BInput) -> Option<Self> {
        let mut b = ControllerB { target, target_dir, can_move: input.can_move, ..Default::default() };
        b.plan_valid = b.plan(s, paths, input.position);
        b.plan_valid.then_some(b)
    }

    /// `82467088`: solve and accept.
    fn plan(&mut self, s: &ControllerBSettings, paths: &dyn PathService, position: [f32; 3]) -> bool {
        self.path.clear();
        self.waypoint = 0;
        let Some(path) = paths.solve(position, self.target) else { return false };
        let ok = match path.last() {
            Some(last) => within(sub(self.target, last.entry), s.plan_path_h, s.plan_path_v),
            None => within(sub(self.target, position), s.plan_direct_h, s.plan_direct_v),
        };
        if !ok {
            return false;
        }
        self.aim = path.first().map_or(self.target, |e| e.entry);
        self.aim_vector = flat(sub(self.aim, position));
        self.path = path;
        true
    }

    /// `82467E48`.
    fn arrived(&self, s: &ControllerBSettings, position: [f32; 3]) -> bool {
        self.waypoint >= self.path.len()
            && within(sub(self.target, position), s.arrive_h, s.arrive_v)
            && signed_angle(self.body, self.target_dir).abs() < s.arrive_heading
    }

    /// `82467968`: P within the leg's corridor (closest point on the segment).
    fn on_leg(s: &ControllerBSettings, a: [f32; 3], b: [f32; 3], p: [f32; 3]) -> bool {
        let ab = sub(b, a);
        let l2 = dot(ab, ab);
        let t = if l2 < s.leg_epsilon_sq { 0.0 } else { (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0) };
        let c = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
        within(sub(p, c), s.leg_h, s.leg_v)
    }

    /// `82467830`: the waypoint advance and the new aim.
    fn advance(&mut self, s: &ControllerBSettings, p: [f32; 3]) {
        let n = self.path.len();
        if self.waypoint < n {
            let last = self.path[n - 1];
            if Self::on_leg(s, last.exit, self.target, p) || Self::on_leg(s, last.entry, last.exit, p) {
                self.waypoint = n;
            } else {
                let e = self.path[self.waypoint];
                if Self::on_leg(s, e.entry, e.exit, p) {
                    self.waypoint += 1;
                }
            }
        }
        self.aim = if self.waypoint < n { self.path[self.waypoint].entry } else { self.target };
        self.aim_vector = flat(sub(self.aim, p));
    }

    /// `82467C90`.
    fn steer(&mut self, s: &ControllerBSettings, p: [f32; 3]) -> f32 {
        let mut desired = self.aim_vector;
        if self.waypoint >= self.path.len() && within(sub(self.target, p), s.line_up_h, s.line_up_v) {
            desired = self.target_dir;
        }
        // 82467C90 passes (desired, B+112): positive when the body lies counter-clockwise of the desired heading.
        let err = signed_angle(desired, self.body);
        self.heading_error = err;
        let m = err.abs();
        let steer = if err == 0.0 { 0.0 } else { (if m < s.steer_full_lock { m * s.steer_gain } else { 1.0 }).copysign(err) };
        self.last_steer = steer;
        steer
    }

    /// `82467B00`.
    fn throttle(&mut self, s: &ControllerBSettings, p: [f32; 3]) -> Option<f32> {
        let l = dot(self.aim_vector, self.aim_vector).sqrt();
        let along = if l > 0.0 { dot(self.body, self.aim_vector) / l } else { 0.0 };
        let d = sub(self.aim, p);
        let factor = if along < s.slow_along { along * s.slow_gain } else { 1.0 };
        let mut f = dot(d, d) * s.throttle_gain * factor;
        if f < s.throttle_floor && along > s.floor_along {
            f = s.throttle_floor;
        }
        if self.stuck.no_move > s.stall_ticks && self.last_steer.abs() > s.stall_steer {
            f = 0.0;
        }
        self.push_mode = 1;
        (f > 0.0).then_some(f.min(s.magnitude_cap))
    }

    /// `824712E0`.
    /// Mode 1 (get off) presses while on the board, mode 2 (get on) while off it; never while toggling; the count
    /// is tested, reset on a press, then always counted up.
    fn push(&mut self, s: &ControllerBSettings, byte_161: bool, byte_160: bool) -> bool {
        let press = self.push_mode != 0
            && self.push_ticks > s.push_period
            && !byte_160
            && ((self.push_mode == 1 && !byte_161) || (self.push_mode == 2 && byte_161));
        if press {
            self.push_ticks = 0;
        }
        self.push_ticks += 1;
        press
    }

    /// One tick (`824675C8`).
    pub fn tick(&mut self, s: &ControllerBSettings, paths: &dyn PathService, i: &BInput) -> BTick {
        if i.abort_flag {
            let keep = (self.target, self.target_dir);
            *self = ControllerB { target: keep.0, target_dir: keep.1, can_move: i.can_move, abort: true, ..Default::default() };
        }
        if i.state_hand_back || self.abort {
            self.abort = false;
            return BTick::HandBack { reason: HandBack::Aborted, wipe_out: false };
        }
        if i.online {
            return BTick::Idle;
        }
        let wipe_out = self.stop_request || self.stuck.stuck;
        // 82467718: the latch, a re-plan, arrival, the waypoint advance and the stuck monitor.
        let mut replan = false;
        if !self.can_move {
            self.can_move = i.can_move;
            replan = self.can_move;
        }
        if !replan && self.plan_valid && !paths.still_valid(&self.path, i.position) {
            replan = true;
        }
        if replan {
            self.plan_valid = self.plan(s, paths, i.position);
        }
        if !self.plan_valid {
            return BTick::HandBack { reason: HandBack::NoPlan, wipe_out };
        }
        if self.arrived(s, i.position) {
            return BTick::HandBack { reason: HandBack::Arrived, wipe_out };
        }
        self.body = i.body;
        self.advance(s, i.position);
        let waypoint = self.waypoint;
        self.stuck.tick(s, i.position, i.facing, waypoint, self.can_move);
        let push = self.push(s, i.byte_161, i.byte_160);
        let steer = self.steer(s, i.position);
        let throttle = self.throttle(s, i.position);
        BTick::Controls(BControls { steer, throttle, mag_held: throttle.unwrap_or(steer), wipe_out, push })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path with the goal as its only corner (what the navmesh returns on open ground).
    struct Corner;
    impl PathService for Corner {
        fn solve(&self, _from: [f32; 3], to: [f32; 3]) -> Option<Vec<PathElement>> {
            Some(vec![PathElement { entry: to, exit: to }])
        }
    }

    fn input(p: [f32; 3], body: [f32; 3]) -> BInput {
        BInput { position: p, body, can_move: true, ..Default::default() }
    }

    #[test]
    fn the_direct_plan_accepts_only_close_targets() {
        let s = ControllerBSettings::default();
        let i = input([0.0; 3], [0.0, 0.0, 1.0]);
        assert!(ControllerB::activate(&s, &DirectPath, [0.0, 2.9, 1.49], [0.0, 0.0, 1.0], &i).is_some());
        assert!(ControllerB::activate(&s, &DirectPath, [0.0, 0.0, 1.51], [0.0, 0.0, 1.0], &i).is_none());
        assert!(ControllerB::activate(&s, &DirectPath, [0.0, 3.01, 0.5], [0.0, 0.0, 1.0], &i).is_none());
    }

    #[test]
    fn steer_is_linear_to_full_lock_at_thirty_degrees_and_throttle_follows_the_distance() {
        let s = ControllerBSettings::default();
        let mut b = ControllerB::activate(&s, &Corner, [0.0, 0.0, 1.4], [0.0, 0.0, 1.0], &input([0.0; 3], [0.0, 0.0, 1.0])).unwrap();
        // Facing 25 degrees off the aim (outside the pi/8 arrival heading): 25 / 30 of full lock.
        let a = 25f32.to_radians();
        let body = [-a.sin(), 0.0, a.cos()];
        let BTick::Controls(c) = b.tick(&s, &Corner, &input([0.0; 3], body)) else { panic!() };
        assert!((c.steer.abs() - 25.0 / 30.0).abs() < 1e-4, "{}", c.steer);
        // Throttle: 0.5 x 1.4^2 = 0.98 (under the 1.0 cap), along cos 25 > 0.25.
        assert!((c.throttle.unwrap() - 0.98).abs() < 1e-4);
        // 60 degrees off: full lock.
        let a = 60f32.to_radians();
        let BTick::Controls(c) = b.tick(&s, &Corner, &input([0.0; 3], [-a.sin(), 0.0, a.cos()])) else { panic!() };
        assert!((c.steer.abs() - 1.0).abs() < 1e-6);
        // Facing away (along < 0.25): the throttle scales by along x 4, so none when facing backwards.
        let BTick::Controls(c) = b.tick(&s, &Corner, &input([0.0; 3], [0.0, 0.0, -1.0])) else { panic!() };
        assert_eq!(c.throttle, None);
    }

    #[test]
    fn it_hands_back_on_arrival_lined_up_with_the_node() {
        let s = ControllerBSettings::default();
        let mut b = ControllerB::activate(&s, &Corner, [0.0, 0.0, 1.4], [0.0, 0.0, 1.0], &input([0.0; 3], [0.0, 0.0, 1.0])).unwrap();
        // Within 2.4 m but facing across the line: not yet.
        assert!(matches!(b.tick(&s, &Corner, &input([0.0, 0.0, 1.0], [1.0, 0.0, 0.0])), BTick::Controls(_)));
        // Arrival reads last tick's heading (`B+112` is stored after the test): lined up on this tick, handed back on
        // the next.
        assert!(matches!(b.tick(&s, &Corner, &input([0.0, 0.0, 1.0], [0.1, 0.0, 1.0])), BTick::Controls(_)));
        assert_eq!(b.tick(&s, &Corner, &input([0.0, 0.0, 1.0], [0.1, 0.0, 1.0])), BTick::HandBack { reason: HandBack::Arrived, wipe_out: false });
    }

    #[test]
    fn a_direct_plan_hands_back_on_its_first_tick_and_steer_turns_towards_the_aim() {
        let s = ControllerBSettings::default();
        // No waypoints and B+112 still zero: angle 0, so arrival holds at once (retail 8296EC98 returns 0).
        let mut b = ControllerB::activate(&s, &DirectPath, [0.0, 0.0, 1.4], [1.0, 0.0, 0.0], &input([0.0; 3], [0.0, 0.0, 1.0])).unwrap();
        assert_eq!(b.tick(&s, &DirectPath, &input([0.0; 3], [0.0, 0.0, 1.0])), BTick::HandBack { reason: HandBack::Arrived, wipe_out: false });
        // Aim straight ahead (+z), body turned towards +x: (aim x body) . up = 1 x 0.5 > 0, a positive steer
        // (OB_Steer; ours publishes ob_Turn as Processed +2680 = -OB_Steer).
        let mut b = ControllerB::activate(&s, &Corner, [0.0, 0.0, 4.0], [0.0, 0.0, 1.0], &input([0.0; 3], [1.0, 0.0, 0.0])).unwrap();
        let BTick::Controls(c) = b.tick(&s, &Corner, &input([0.0; 3], [0.5, 0.0, 1.0])) else { panic!() };
        assert!(c.steer > 0.0, "{c:?}");
    }

    #[test]
    fn a_stalled_skater_is_stuck_and_asks_to_wipe_out() {
        let s = ControllerBSettings::default();
        let p = [0.0, 0.0, 0.0];
        let mut b = ControllerB::activate(&s, &Corner, [0.0, 0.0, 1.4], [1.0, 0.0, 0.0], &input(p, [0.0, 0.0, 1.0])).unwrap();
        let mut wiped = None;
        for k in 0..200 {
            if let BTick::Controls(c) = b.tick(&s, &Corner, &input(p, [0.0, 0.0, 1.0])) {
                if c.wipe_out {
                    wiped = Some(k);
                    break;
                }
            }
        }
        // Stall counts from the first tick that neither moves nor turns (tick 0 moves from the reset's zero anchor
        // only when away from the origin; here it starts at the origin): stuck after 91 stalled ticks, the request
        // goes out on the next tick.
        assert_eq!(wiped, Some(91));
    }

    #[test]
    fn push_pairs_every_five_ticks_while_not_rolling_ready() {
        let s = ControllerBSettings::default();
        let mut b = ControllerB::activate(&s, &Corner, [0.0, 0.0, 1.4], [0.0, 0.0, 1.0], &input([0.0; 3], [0.0, 0.0, 1.0])).unwrap();
        let mut presses = Vec::new();
        for k in 0..17 {
            let mut i = input([0.0, 0.0, 0.01 * k as f32], [1.0, 0.0, 0.0]);
            i.byte_161 = false;
            if let BTick::Controls(c) = b.tick(&s, &Corner, &i) {
                if c.push {
                    presses.push(k);
                }
            }
        }
        // The count is tested before it counts up (0 on tick 0): the first press on tick 5, then every 5 ticks.
        assert_eq!(presses, vec![5, 10, 15]);
    }

    #[test]
    fn online_does_nothing_and_abort_hands_back() {
        let s = ControllerBSettings::default();
        let mut b = ControllerB::activate(&s, &Corner, [0.0, 0.0, 1.4], [0.0, 0.0, 1.0], &input([0.0; 3], [0.0, 0.0, 1.0])).unwrap();
        let mut i = input([0.0; 3], [1.0, 0.0, 0.0]);
        i.online = true;
        assert_eq!(b.tick(&s, &Corner, &i), BTick::Idle);
        i.online = false;
        i.abort_flag = true;
        assert_eq!(b.tick(&s, &Corner, &i), BTick::HandBack { reason: HandBack::Aborted, wipe_out: false });
    }
}
