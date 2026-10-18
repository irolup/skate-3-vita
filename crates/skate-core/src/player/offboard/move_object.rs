//! Move Object (off-board grab and push, physics state 502): the per-tick
//! command retail 82D45318 sends to the held dynamic object, as a pure
//! function over plain data (no ECS, no I/O).
//!
//! Retail mechanism [code, TU3 recomp]: every 60 Hz tick the held object gets
//! a bounded command, a horizontal linear term plus a yaw term only, from two
//! velocity-tracking controllers (PhysicsControllerData, update 82D4E118). It
//! is never position- or velocity-forced, and there is no pitch, roll or
//! vertical term (lift gain FDE807D9B85A6AC2 = 0), so gravity, contacts and
//! tipping stay with the normal rigid-body simulation. The skater follows the
//! object's grab edge (82D45D30), not the other way round.
//!
//! Every tuning value is data: attribute collection class
//! `3EDA5B140604613D`, key `default` (loaded by skate-game); the
//! [`MoveObjectTuning::default`] values are the stock values for tests and a
//! missing collection. Image constants (slew 4.0 per tick 0x82257308, filter
//! 0.9 / 0.1, blocked decay 0.95 and gain 0.5, lever coupling -0.25
//! 0x8208ED00) are fields too, so a mod can reach every number.
//!
//! Multiplayer: [`MoveObjectController`] is the whole per-carrier state, plain
//! `Copy` data with a flat `to_array` / `from_array` form; the command is a
//! deterministic function of (tuning, state, input), run once per fixed tick.
use crate::point_graph::PointGraph;

mod held_record;
pub use held_record::{HeldGrip, RecordFrame, begin_grip, continue_grip, record_frame};
mod held_update;
pub use held_update::{FrameBlend, HandIk, Rebind, RebindInput, RebindState, RebindTuning, hand_points, seed_anchor};

/// `Sk8::Physics::PhysicsControllerData`: four floats. 82D4E118 [code]:
/// `filtered = (1 - filter) * filtered + filter * error`;
/// `output += proportional * error + filtered_gain * filtered + derivative * (error - previous)`.
/// The output accumulates; the clamp is not inside the controller, but the
/// caller writes the bounded command back into the output (anti-windup, see
/// [`command`] steps 8 and 10).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerGains {
    pub proportional: f32,
    pub filtered: f32,
    pub derivative: f32,
    pub filter: f32,
}

impl ControllerGains {
    /// Stock DF79539DBDA006EE / B46764285AD1DC5F: 20, 0, 40, 0.1.
    pub const STOCK: Self = Self { proportional: 20.0, filtered: 0.0, derivative: 40.0, filter: 0.1 };

    /// From the four big-endian words of the attribute field.
    pub fn from_words(words: [u32; 4]) -> Self {
        let f = words.map(f32::from_bits);
        Self { proportional: f[0], filtered: f[1], derivative: f[2], filter: f[3] }
    }
}

/// State of one controller lane (82D4E118 block: +16 output, +32 previous
/// error, +48 filtered error).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControllerLane {
    pub output: f32,
    pub previous: f32,
    pub filtered: f32,
}

impl ControllerLane {
    pub fn update(&mut self, gains: ControllerGains, error: f32) -> f32 {
        self.filtered = (1.0 - gains.filter) * self.filtered + gains.filter * error;
        let derivative = error - self.previous;
        self.previous = error;
        self.output += gains.proportional * error + gains.filtered * self.filtered + gains.derivative * derivative;
        self.output
    }
}

/// All Move Object tuning (class 3EDA5B140604613D `default`, plus the image
/// constants 82D45318 reads). Field docs name the attribute hash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveObjectTuning {
    /// 2258076B612569A9: m/s target pushing (OB_ObjectMvZ > 0).
    pub push_speed: f32,
    /// F1C038722EC7D0C6: m/s target pulling.
    pub pull_speed: f32,
    /// 096A4FA6489E5541: m/s target sideways along the edge.
    pub side_speed: f32,
    /// E4FF0185DA44CDBD: |lever| (m) -> rotation demand from pushing.
    pub lever_rotation: PointGraph<8>,
    /// BFB3BEF0BB2661C0: |lever| -> yaw-rate factor.
    pub lever_yaw: PointGraph<8>,
    /// 57D37D696363167E: mass (kg) -> speed scale.
    pub mass_speed: PointGraph<8>,
    /// EABFCC79873A2859: yaw inertia -> yaw gain.
    pub inertia_yaw_gain: PointGraph<8>,
    /// AD327350D151B1E3: yaw command clamp.
    pub yaw_clamp: f32,
    /// 791421DDAF54C2D5: linear command clamp (length).
    pub linear_clamp: f32,
    /// 557FA142008FD7CE: heading re-latch threshold (rad).
    pub relatch: f32,
    /// FDE807D9B85A6AC2: vertical (lift) gain. Stock 0; kept so a mod sees it,
    /// the port applies no vertical command at 0.
    pub lift_gain: f32,
    /// DF79539DBDA006EE: linear controller.
    pub linear_controller: ControllerGains,
    /// B46764285AD1DC5F: yaw controller.
    pub yaw_controller: ControllerGains,
    /// Image 0x82257308: max change of the sent linear command per tick.
    pub slew_per_tick: f32,
    /// Image 0x820997B8 / 0x820641A8: +640 smoothing (keep, new share).
    pub command_filter: f32,
    /// Image 0x82072818: blocking-normal decay per tick.
    pub blocked_decay: f32,
    /// Image 0x8209975C: share of the skater contact normal added per tick.
    pub blocked_gain: f32,
    /// Blocked bit 0x10 while |blocking|^2 exceeds this (2.5e-5).
    pub blocked_threshold_sq: f32,
    /// Image 0x8208ED00: centre speed from the turn about the grip.
    pub lever_coupling: f32,
    /// |yaw target| below this keeps the heading latch (0.1).
    pub turning_threshold: f32,
    /// Yaw-rate feedback: the per-tick change of the object's facing (+384 ->
    /// now, 8296EC98) times this is the measured yaw rate the yaw controller
    /// subtracts from its target. Retail 60 = 1/dt, image 0x822F860C loaded at
    /// 0x82D45C08 [code]. 0 (mods only) turns the feedback off.
    pub yaw_rate_feedback: f32,
    /// Target velocity scale while the held record's +272 is non-zero
    /// (82D45318, `cmpwi` on state+992, 2.0 at image 0x82060C50) [code].
    /// Record+272 comes from the DMO's type data (82C4B960: data+312 set ->
    /// 1 or 2), see [`MoveObjectInput::record_272`].
    pub record_272_speed_scale: f32,
    /// Hold qualification while grabbing (82D44A10 -> 82E08EE8 at the grip
    /// distance +1128), from physics_state_offboard `default` [data]: grab
    /// box half extents GrabBoxSizeGrabbing (+0), its offset GrabBoxOffset
    /// (+32), GrabSplineAngleLimitGrabbing (+452, degrees, approach angle) and
    /// GrabSplineMaxAngleToHorizontalGrabbing (+436, degrees, edge slope).
    pub hold_box_extents: [f32; 3],
    pub hold_box_offset: [f32; 3],
    pub hold_angle_limit: f32,
    pub hold_max_angle_to_horizontal: f32,
    /// Grip end exclusion (82D444A0 full begin) [data]: the grip on the held
    /// record is clamped to [h, length - h], h = min(this, length / 2);
    /// physics_state_offboard `default` GrabSplineEndExclusion (+444, 0.25).
    pub grab_end_exclusion: f32,
    /// Skater follow (82D44A10 before 82BDF268) [code]: the follow point
    /// (+416) steps toward the target frame's edge point (+192) moved
    /// `follow_reach` (0.65, image 0x820BB0EC) along the latched frame toward
    /// the skater, at the skater root height plus `follow_height` (0.72,
    /// 0x8220E144). The step is the anchor velocity (+624) x `tick` plus the
    /// rest clamped to `follow_step` m (0.1, 0x820641A8; 82BD41B0).
    pub follow_reach: f32,
    pub follow_height: f32,
    pub follow_step: f32,
    /// Anchor velocity (+624, 82D46610) [code + data]: the anchor is the grip
    /// point moved `anchor_reach` toward the skater (96ECC98838ECCC11 = 0.7,
    /// this collection), y dropped; v = keep x v + new x (anchor change / tick)
    /// with keep 0.85 (0x822250F8) and new 0.15 (0x820994B4).
    pub anchor_reach: f32,
    pub anchor_velocity_keep: f32,
    pub anchor_velocity_new: f32,
    /// Retail tick (1/60, image 0x820849C8) used by the follow step.
    pub tick: f32,
    /// Hand IK of the held update (82D46610 sets +1200 bit 0x40, 82D45008 steps the weight +1132) [code + data]:
    /// on once `hand_ik_enter` (+1180, 5E35DB02BE697A58 = 0.7216) > 0, the state time is at most `hand_ik_window`
    /// (the larger x end of curves 1348E9A1F213B42D / 702F25BA3A5AAA56, 1.0) and 1 - `hand_ik_curve`
    /// (702F25BA3A5AAA56) at the state time exceeds `hand_ik_threshold` (0.1, 0x820641A8). The weight moves
    /// `hand_ik_rate` (0.2, 0x82099280) per tick; the targets are clamped to `hand_ik_reach` (0.65, 0x820BB0EC)
    /// around the animated hands (82BD9728 / 82BD97D0).
    pub hand_ik_enter: f32,
    pub hand_ik_window: f32,
    pub hand_ik_curve: PointGraph<8>,
    pub hand_ik_threshold: f32,
    pub hand_ik_rate: f32,
    pub hand_ik_reach: f32,
}

const fn graph(x: [f32; 8], y: [f32; 8]) -> PointGraph<8> {
    PointGraph { x, y }
}

impl Default for MoveObjectTuning {
    /// Stock values (section 3 of the Move Object research note). The game
    /// loads the same values from the attribute collection; these are the
    /// fallback for tests and a missing collection.
    fn default() -> Self {
        Self {
            push_speed: 3.0,
            pull_speed: 2.0,
            side_speed: 2.5,
            lever_rotation: graph([0.0, 0.2378, 0.5505, 0.8209, 1.1466, 1.5880, 1.7932, 2.0], [0.0, 0.0, 0.07, 0.18, 0.39, 0.91, 1.0, 1.0]),
            lever_yaw: graph([0.0, 0.29, 0.41, 0.53, 0.62, 0.69, 0.84, 1.0], [1.0, 1.0, 0.96, 0.91, 0.83, 0.77, 0.65, 0.53]),
            mass_speed: graph([0.0, 37.13, 58.63, 76.22, 90.55, 148.21, 184.0, 200.0], [1.0, 1.0, 1.0, 1.0, 1.0, 0.9, 0.68, 0.51]),
            inertia_yaw_gain: graph([0.0, 9.12, 114.66, 170.69, 400.0, 543.32, 638.0, 800.0], [2.0, 2.0, 1.52, 1.25, 1.25, 0.99, 0.88, 0.66]),
            yaw_clamp: 6.0,
            linear_clamp: 20.0,
            relatch: 0.1,
            lift_gain: 0.0,
            linear_controller: ControllerGains::STOCK,
            yaw_controller: ControllerGains::STOCK,
            slew_per_tick: 4.0,
            command_filter: 0.1,
            blocked_decay: 0.95,
            blocked_gain: 0.5,
            blocked_threshold_sq: 2.5e-5,
            lever_coupling: -0.25,
            turning_threshold: 0.1,
            yaw_rate_feedback: 60.0,
            record_272_speed_scale: 2.0,
            hold_box_extents: [0.9, 0.8, 1.01],
            hold_box_offset: [0.0, 1.0, 0.1],
            hold_angle_limit: 80.0,
            hold_max_angle_to_horizontal: 50.0,
            grab_end_exclusion: 0.25,
            follow_reach: 0.65,
            follow_height: 0.72,
            follow_step: 0.1,
            anchor_reach: 0.7,
            anchor_velocity_keep: 0.85,
            anchor_velocity_new: 0.15,
            tick: 1.0 / 60.0,
            hand_ik_enter: 0.7216,
            hand_ik_window: 1.0,
            hand_ik_curve: graph([0.0, 0.0205, 0.0969, 0.1562, 0.2018, 0.2736, 0.3169, 0.35], [1.0, 1.0, 0.9214, 0.7071, 0.5071, 0.1786, 0.0393, 0.0]),
            hand_ik_threshold: 0.1,
            hand_ik_rate: 0.2,
            hand_ik_reach: 0.65,
        }
    }
}

impl MoveObjectTuning {
    /// 82D444A0: speed scale from the object's mass (1 / record+240).
    pub fn speed_scale(&self, mass: f32) -> f32 {
        self.mass_speed.evaluate(mass)
    }

    /// 82D444A0: yaw gain (+1160) from the yaw inertia (1 / record+228).
    pub fn yaw_gain(&self, yaw_inertia: f32) -> f32 {
        self.inertia_yaw_gain.evaluate(yaw_inertia)
    }

    /// Every scalar finite, speeds / clamps non-negative; else the field falls
    /// back to `fallback` (mod values are validated, never trusted).
    pub fn sanitized(self, fallback: &Self) -> Self {
        let ok = |v: f32, d: f32| if v.is_finite() && v >= 0.0 { v } else { d };
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let gains = |g: ControllerGains, d: ControllerGains| ControllerGains {
            proportional: fin(g.proportional, d.proportional),
            filtered: fin(g.filtered, d.filtered),
            derivative: fin(g.derivative, d.derivative),
            filter: if g.filter.is_finite() && (0.0..=1.0).contains(&g.filter) { g.filter } else { d.filter },
        };
        let curve = |c: PointGraph<8>, d: PointGraph<8>| {
            let finite = c.x.iter().chain(c.y.iter()).all(|v| v.is_finite());
            let ordered = c.x.windows(2).all(|p| p[0] <= p[1]);
            if finite && ordered { c } else { d }
        };
        Self {
            push_speed: ok(self.push_speed, fallback.push_speed),
            pull_speed: ok(self.pull_speed, fallback.pull_speed),
            side_speed: ok(self.side_speed, fallback.side_speed),
            lever_rotation: curve(self.lever_rotation, fallback.lever_rotation),
            lever_yaw: curve(self.lever_yaw, fallback.lever_yaw),
            mass_speed: curve(self.mass_speed, fallback.mass_speed),
            inertia_yaw_gain: curve(self.inertia_yaw_gain, fallback.inertia_yaw_gain),
            yaw_clamp: ok(self.yaw_clamp, fallback.yaw_clamp),
            linear_clamp: ok(self.linear_clamp, fallback.linear_clamp),
            relatch: ok(self.relatch, fallback.relatch),
            lift_gain: fin(self.lift_gain, fallback.lift_gain),
            linear_controller: gains(self.linear_controller, fallback.linear_controller),
            yaw_controller: gains(self.yaw_controller, fallback.yaw_controller),
            slew_per_tick: ok(self.slew_per_tick, fallback.slew_per_tick),
            command_filter: if (0.0..=1.0).contains(&self.command_filter) { self.command_filter } else { fallback.command_filter },
            blocked_decay: if (0.0..=1.0).contains(&self.blocked_decay) { self.blocked_decay } else { fallback.blocked_decay },
            blocked_gain: ok(self.blocked_gain, fallback.blocked_gain),
            blocked_threshold_sq: ok(self.blocked_threshold_sq, fallback.blocked_threshold_sq),
            lever_coupling: fin(self.lever_coupling, fallback.lever_coupling),
            turning_threshold: ok(self.turning_threshold, fallback.turning_threshold),
            yaw_rate_feedback: ok(self.yaw_rate_feedback, fallback.yaw_rate_feedback),
            record_272_speed_scale: ok(self.record_272_speed_scale, fallback.record_272_speed_scale),
            hold_box_extents: if self.hold_box_extents.iter().all(|v| v.is_finite() && *v >= 0.0) { self.hold_box_extents } else { fallback.hold_box_extents },
            hold_box_offset: if self.hold_box_offset.iter().all(|v| v.is_finite()) { self.hold_box_offset } else { fallback.hold_box_offset },
            hold_angle_limit: ok(self.hold_angle_limit, fallback.hold_angle_limit),
            hold_max_angle_to_horizontal: ok(self.hold_max_angle_to_horizontal, fallback.hold_max_angle_to_horizontal),
            grab_end_exclusion: ok(self.grab_end_exclusion, fallback.grab_end_exclusion),
            follow_reach: fin(self.follow_reach, fallback.follow_reach),
            follow_height: fin(self.follow_height, fallback.follow_height),
            follow_step: ok(self.follow_step, fallback.follow_step),
            anchor_reach: fin(self.anchor_reach, fallback.anchor_reach),
            anchor_velocity_keep: if (0.0..=1.0).contains(&self.anchor_velocity_keep) { self.anchor_velocity_keep } else { fallback.anchor_velocity_keep },
            anchor_velocity_new: if (0.0..=1.0).contains(&self.anchor_velocity_new) { self.anchor_velocity_new } else { fallback.anchor_velocity_new },
            tick: if self.tick.is_finite() && self.tick > 0.0 { self.tick } else { fallback.tick },
            hand_ik_enter: fin(self.hand_ik_enter, fallback.hand_ik_enter),
            hand_ik_window: fin(self.hand_ik_window, fallback.hand_ik_window),
            hand_ik_curve: curve(self.hand_ik_curve, fallback.hand_ik_curve),
            hand_ik_threshold: fin(self.hand_ik_threshold, fallback.hand_ik_threshold),
            hand_ik_rate: ok(self.hand_ik_rate, fallback.hand_ik_rate),
            hand_ik_reach: ok(self.hand_ik_reach, fallback.hand_ik_reach),
        }
    }
}

/// Per-carrier Move Object state (the state object's +640 / +656 / +672 /
/// +368 / +384 / +1008 / +1088 parts). Plain data: a host can snapshot and replay it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveObjectController {
    /// +1008 linear controller, x / y / z lanes.
    pub linear: [ControllerLane; 3],
    /// +1088 yaw controller.
    pub yaw: ControllerLane,
    /// +672: slew-limited linear command actually sent.
    pub sent: [f32; 3],
    /// +640: smoothed sent command.
    pub filtered: [f32; 3],
    /// +656: smoothed blocking normal.
    pub blocking: [f32; 3],
    /// +368 / +400: latched push frame (forward into the object, side along
    /// the edge) and the object heading it was latched at.
    pub latched_forward: [f32; 3],
    pub latched_side: [f32; 3],
    pub latched_yaw: f32,
    pub latched: bool,
    /// +384: the object's facing on the previous tick (overwritten every
    /// tick, 0x82D45C0C), as a heading about +Y. `facing_valid` false = the
    /// zero vector retail starts from, which measures a zero rate (8296EC98
    /// returns 0 for vectors shorter than 1e-2).
    pub facing_yaw: f32,
    pub facing_valid: bool,
}

/// Length of [`MoveObjectController::to_array`].
pub const CONTROLLER_FLOATS: usize = 33;

impl MoveObjectController {
    /// Flat, ordered form for serialisation (snapshot / replay / network).
    pub fn to_array(&self) -> [f32; CONTROLLER_FLOATS] {
        let mut out = [0.0; CONTROLLER_FLOATS];
        let mut i = 0;
        let mut put = |v: f32| {
            out[i] = v;
            i += 1;
        };
        for lane in self.linear.iter().chain(std::iter::once(&self.yaw)) {
            put(lane.output);
            put(lane.previous);
            put(lane.filtered);
        }
        for v in [self.sent, self.filtered, self.blocking, self.latched_forward, self.latched_side] {
            v.iter().for_each(|&x| put(x));
        }
        put(self.latched_yaw);
        put(if self.latched { 1.0 } else { 0.0 });
        put(0.0); // a[29], a[30] reserved
        put(0.0); // (zero) for the frame blend timer / rate (+1164 / +1168).
        put(self.facing_yaw);
        put(if self.facing_valid { 1.0 } else { 0.0 });
        out
    }

    pub fn from_array(a: [f32; CONTROLLER_FLOATS]) -> Self {
        let lane = |i: usize| ControllerLane { output: a[i], previous: a[i + 1], filtered: a[i + 2] };
        let v3 = |i: usize| [a[i], a[i + 1], a[i + 2]];
        Self {
            linear: [lane(0), lane(3), lane(6)],
            yaw: lane(9),
            sent: v3(12),
            filtered: v3(15),
            blocking: v3(18),
            latched_forward: v3(21),
            latched_side: v3(24),
            latched_yaw: a[27],
            latched: a[28] != 0.0,
            // a[29], a[30] reserved (zero) for the frame blend timer / rate (+1164 / +1168).
            facing_yaw: a[31],
            facing_valid: a[32] != 0.0,
        }
    }
}

/// One tick's inputs (world space, Y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveObjectInput {
    /// OB_ObjectMvZ / OB_ObjectMvX / OB_ObjectMvRot (Player+2784/2788/2792).
    pub move_z: f32,
    pub move_x: f32,
    pub move_rotation: f32,
    /// Horizontal unit normal of the grab edge pointing from the skater into
    /// the object.
    pub forward: [f32; 3],
    /// Current grip point on the grab edge (state +256).
    pub grip: [f32; 3],
    /// Object centre (record +160) and velocity (record +128).
    pub center: [f32; 3],
    pub velocity: [f32; 3],
    /// Object heading about +Y (rad).
    pub heading: f32,
    /// 1 / record+240 and 1 / record+228.
    pub mass: f32,
    pub yaw_inertia: f32,
    /// Skater's own contact normal (Player+16304), zero when free.
    pub contact_normal: [f32; 3],
    /// Held record +272 non-zero (DMO type data, 82C4B960): the target
    /// velocity is scaled by `record_272_speed_scale`.
    pub record_272: bool,
}

/// One tick's output: the command plus what HELD_PROP logs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveObjectCommand {
    /// Horizontal linear command (y always 0, lift gain 0).
    pub linear: [f32; 3],
    /// Yaw command about +Y (rad/s per tick of controller output).
    pub yaw: f32,
    pub lever: f32,
    pub rotation_demand: f32,
    /// Yaw-rate target (rad/s about +Y, positive turns +Z toward +X).
    pub yaw_target: f32,
    /// Target velocity before the controller (+1196 = its length).
    pub target_velocity: [f32; 3],
    pub blocked: bool,
    /// Heading-latch drift (rad, 0 when re-latched or turning): only decides
    /// the re-latch, it does not enter the yaw error.
    pub drift: f32,
    /// Measured yaw rate fed back into the yaw controller (rad/s, +1192 holds
    /// its magnitude).
    pub yaw_rate: f32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn wrap(angle: f32) -> f32 {
    let pi = std::f32::consts::PI;
    (angle + pi).rem_euclid(std::f32::consts::TAU) - pi
}

fn clamp_length(v: [f32; 3], limit: f32) -> [f32; 3] {
    let l = dot(v, v).sqrt();
    if l > limit && l > 0.0 { v.map(|c| c * limit / l) } else { v }
}

/// Side axis of a horizontal forward: the skater's right, (0,0,1) -> (1,0,0).
pub fn side_of(forward: [f32; 3]) -> [f32; 3] {
    [forward[2], 0.0, -forward[0]]
}

/// Port of 82D45318 steps 1 to 10, signs as in the code: lever = dot(-edge,
/// centre - grip), rot = min(1, sign(lever) curve(|lever|) MvZ + MvRot),
/// w = -(curve(|lever|) rot gain), fwd += 0.25 lever w, yaw command sent as
/// (0, out, 0) about +Y (slot 9 angular sink, right-handed like our world).
/// The yaw error is the target minus the measured yaw rate (the facing change
/// since the previous tick x 60, same rotation sense: +Y turns +Z toward +X),
/// so the feedback is negative. Edge direction: retail builds
/// +320 from the hand points (82D45D30, sense not traced); taking it as
/// `side_of(forward)` is the only choice for which an off-centre push turns
/// the object the way the push torque does (r x F about +Y). With it a
/// positive OB_ObjectMvRot gives a negative yaw rate (clockwise seen from
/// above, a right turn for the skater). NOT RETAIL YET: the edge sense.
pub fn command(tuning: &MoveObjectTuning, state: &mut MoveObjectController, input: &MoveObjectInput) -> MoveObjectCommand {
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let (mz, mx, mr) = (finite(input.move_z), finite(input.move_x), finite(input.move_rotation));
    let side = side_of(input.forward);
    // 1. Lever arm [code]: lever = dot(-edge_dir (+320, sign flipped by the
    // vxor), centre (+880) - grip (+256)). Edge direction = `side` (the only
    // choice that makes retail's off-centre turn match the push torque, see
    // the axis note above).
    let offset = [input.center[0] - input.grip[0], 0.0, input.center[2] - input.grip[2]];
    let lever = -dot(side, offset);
    let sign = if lever >= 0.0 { 1.0 } else { -1.0 };
    // 2. Rotation demand (off-centre pushing turns; at the centre it does not).
    let rotation_demand = (sign * tuning.lever_rotation.evaluate(lever.abs()) * mz + mr).min(1.0);
    // 3. Yaw-rate target [code]: f24 = -(curve BFB3(|lever|) x rot x gain +1160).
    let yaw_target = -(tuning.lever_yaw.evaluate(lever.abs()) * rotation_demand * tuning.yaw_gain(input.yaw_inertia));
    // 4. Linear targets.
    let scale = tuning.speed_scale(input.mass);
    let mut along = mz * if mz > 0.0 { tuning.push_speed } else { tuning.pull_speed } * scale;
    let across = mx * tuning.side_speed * scale;
    // [code] fnmsubs: fwd = fwd - (lever x w) x c, c = -0.25 (0x8208ED00).
    along -= lever * yaw_target * tuning.lever_coupling;
    // 5. Heading latch [code, 0x82D455E0..0x82D4573C]: turning (|w| >= 0.1,
    // 0x820641A8) stores the latch every tick (0x82D45714). Otherwise the
    // wrapped angle between the facing and +368 (8296EBB0) is the drift; above
    // the re-latch threshold (557FA142008FD7CE) the latch is stored, below it
    // the store is skipped (0x82D4570C). The drift does not enter the yaw error.
    let mut drift = 0.0;
    if !state.latched || yaw_target.abs() >= tuning.turning_threshold {
        state.latched = true;
        state.latched_yaw = input.heading;
        state.latched_forward = input.forward;
        state.latched_side = side;
    } else {
        drift = wrap(input.heading - state.latched_yaw);
        if drift.abs() > tuning.relatch {
            state.latched_yaw = input.heading;
            state.latched_forward = input.forward;
            state.latched_side = side;
            drift = 0.0;
        }
    }
    // 6. World direction from the latched frame.
    let (f, s) = (state.latched_forward, state.latched_side);
    let mut v = [f[0] * along + s[0] * across, 0.0, f[2] * along + s[2] * across];
    // Record+272 [code, 82D45318 `cmpwi` on +992]: non-zero doubles v
    // (2.0 at 0x82060C50) before the blocking clip and +1196 = |v|.
    if input.record_272 {
        v = v.map(|c| c * tuning.record_272_speed_scale);
    }
    // 7. Blocking normal (decay + skater contact normal) clips the into-wall part.
    let n = input.contact_normal.map(finite);
    state.blocking = std::array::from_fn(|i| tuning.blocked_decay * state.blocking[i] + tuning.blocked_gain * n[i]);
    let blocked = dot(state.blocking, state.blocking) > tuning.blocked_threshold_sq;
    if blocked {
        let l = dot(state.blocking, state.blocking).sqrt();
        let u = state.blocking.map(|c| c / l);
        let into = dot(v, u);
        if into < 0.0 {
            v = std::array::from_fn(|i| v[i] - u[i] * into);
        }
    }
    // 8. Linear controller on the horizontal velocity error, clamp, slew.
    let error = [v[0] - finite(input.velocity[0]), 0.0, v[2] - finite(input.velocity[2])];
    let out: [f32; 3] = std::array::from_fn(|i| state.linear[i].update(tuning.linear_controller, error[i]));
    let clamped = clamp_length(out, tuning.linear_clamp);
    let step = clamp_length(std::array::from_fn(|i| clamped[i] - state.sent[i]), tuning.slew_per_tick);
    state.sent = std::array::from_fn(|i| state.sent[i] + step[i]);
    // Anti-windup [code, 82D45318 after the slew]: the sent command (+672) is
    // also stored over the controller output (+1008 +16), so the accumulator
    // never runs past what was actually sent.
    for i in 0..3 {
        state.linear[i].output = state.sent[i];
    }
    state.filtered = std::array::from_fn(|i| (1.0 - tuning.command_filter) * state.filtered[i] + tuning.command_filter * state.sent[i]);
    // 9. Vertical: lift gain x height error; stock 0 and not applied.
    // 10. Yaw controller [code, 0x82D45BC8..0x82D45CD0]: 8296EC98(+384, now,
    // axis (0, 1, 0) at 0x82139A20) = acos(dot) of the normalised facings,
    // 2 pi - acos when cross(+384, now) . axis < 0 (2 pi at 0x821647F0), 0 for
    // a vector shorter than 1e-2 (|v|^2 vs 1e-4 at 0x8209BE90); wrapped to
    // [-pi, pi) with 1 / 2 pi (0x82139A60) and 2 pi (0x82139A50); then +384 =
    // now (every tick, both latch branches). Rate = wrapped x 60
    // (0x822F860C); error = w - rate. For an upright object the signed angle
    // about +Y is the heading change, which is what the port measures.
    let yaw_rate = if state.facing_valid { wrap(input.heading - state.facing_yaw) * tuning.yaw_rate_feedback } else { 0.0 };
    state.facing_yaw = input.heading;
    state.facing_valid = true;
    let yaw_out = state.yaw.update(tuning.yaw_controller, yaw_target - yaw_rate);
    let yaw = yaw_out.clamp(-tuning.yaw_clamp, tuning.yaw_clamp);
    // Anti-windup [code, 82D45318]: the clamped yaw command is stored back
    // over the yaw controller output (+1088 +16 = +1104).
    state.yaw.output = yaw;
    MoveObjectCommand {
        linear: [state.sent[0], 0.0, state.sent[2]],
        yaw,
        lever,
        rotation_demand,
        yaw_target,
        target_velocity: v,
        blocked,
        drift,
        yaw_rate,
    }
}

/// What entering Move Object does to the skateboard (SkateboardController,
/// the state object's +76 pointer, field +448).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardOnGrab {
    /// `LetGoOfSkateboard` 82D75440, then +448 = 2 (free): the board is
    /// released as a free physics body where it is, with its velocity.
    LetGo,
    /// 82D755E0 (hide: collision off, retrieval reset), then +448 = 3.
    Hide,
    /// +448 untouched (+444 untouched too).
    Keep,
}

/// Board part of the Move Object enter 82D442D0 (0x82D44304..0x82D44364)
/// [code, TU3 recomp]: a switch on SkateboardController+448 (`cmplwi 5`,
/// above 5 skips). 0 (stopped), 1 (held) and 5 (on the ground at the feet)
/// let go and become 2; 4 (being retrieved) hides and becomes 3; 2 (free) and
/// 3 (hidden) are kept. +444 is zeroed before either call, the new state is
/// written after it. So grabbing an object drops a carried board where it is.
pub fn board_on_grab(state_448: u32) -> BoardOnGrab {
    match state_448 {
        0 | 1 | 5 => BoardOnGrab::LetGo,
        4 => BoardOnGrab::Hide,
        _ => BoardOnGrab::Keep,
    }
}

/// A two-point grab record for a straight edge from `a` to `b` (world
/// space) with the approach vector (record+96) `approach`, built by the
/// retail record layout (82585F58 through `Record::from_geometry`, identity
/// object frame). Used for props until their authored grab splines are
/// loaded (DMO physics definition +136, see the Move Object research note).
pub fn edge_record(id: u32, a: [f32; 3], b: [f32; 3], approach: [f32; 3]) -> Option<super::grab_scene::Record> {
    use super::grab_scene::{Descriptor, Geometry, Record, RecordInput};
    let geometry = Geometry {
        id: id.max(1),
        points: vec![[a[0], a[1], a[2], 1.0], [b[0], b[1], b[2], 1.0]],
        approach_vectors: vec![[approach[0], approach[1], approach[2], 0.0]],
        word_60: 0,
    };
    Record::from_geometry(RecordInput {
        descriptor: Descriptor { kind: 2, id: id.max(1) },
        geometry: std::sync::Arc::new(geometry),
        frame: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
        object_vector_128: [0.0; 4],
        assembly: None,
        word_272: 0,
    })
    .ok()
}

/// Held re-qualification of 82D44A10 [code]: CanGrabSpline 82E08EE8 on the
/// held record (state+720) at the grip distance (+1128) along it, from the
/// skater reference point (+272, bone 23), with the grab box placed on the
/// skater frame (Player+192) by 82D2E250 from GrabBoxSizeGrabbing /
/// GrabBoxOffset and the *Grabbing angle limits in degrees (x 0x3C8EFA35).
/// False = retail stops holding (flag 0x20 clears, no command is sent).
/// `skater_frame` rows: right, up, forward, position.
pub fn still_holds(
    tuning: &MoveObjectTuning,
    record: &super::grab_scene::Record,
    reference: [f32; 3],
    grip_distance: f32,
    skater_frame: [[f32; 4]; 4],
) -> bool {
    let v4 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
    let bounds = super::ground_sync::board_bounds(
        skater_frame,
        v4(tuning.hold_box_offset, 0.0),
        v4(tuning.hold_box_extents, 0.0),
    );
    let radians = f32::from_bits(0x3c8e_fa35);
    let limits = super::ground_sync::BoardLimits {
        margin: 0.0,
        angle_a: tuning.hold_angle_limit * radians,
        angle_b: tuning.hold_max_angle_to_horizontal * radians,
    };
    super::grab_scene::qualify_at(record, v4(reference, 1.0), grip_distance, bounds, limits)
}

/// Path B of 82D44A10 (b64): the mode-1 candidate must pass CanGrabSpline
/// 82E08DB8 from the skater reference, its grip clamped by
/// GrabSplineEndExclusion, with the same grabbing box and angles as
/// [`still_holds`].
pub fn can_regrab(
    tuning: &MoveObjectTuning,
    record: &super::grab_scene::Record,
    reference: [f32; 3],
    skater_frame: [[f32; 4]; 4],
) -> bool {
    let v4 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
    let bounds = super::ground_sync::board_bounds(
        skater_frame,
        v4(tuning.hold_box_offset, 0.0),
        v4(tuning.hold_box_extents, 0.0),
    );
    let radians = f32::from_bits(0x3c8e_fa35);
    let limits = super::ground_sync::BoardLimits {
        margin: tuning.grab_end_exclusion,
        angle_a: tuning.hold_angle_limit * radians,
        angle_b: tuning.hold_max_angle_to_horizontal * radians,
    };
    super::grab_scene::qualify(record, v4(reference, 1.0), bounds, limits)
}

/// The skater side of the held update (82D44A10 / 82D46610): where the
/// skater's body is moved to (82BDF268 target, state +416). Plain data per
/// player slot, deterministic per retail tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkaterFollow {
    /// +416: follow point (starts at the body position Skeleton+15872).
    pub point: [f32; 3],
    /// +608: previous anchor (grip point moved toward the skater, y 0).
    pub anchor: [f32; 3],
    /// +624: smoothed anchor velocity (m/s, horizontal).
    pub velocity: [f32; 3],
}

impl SkaterFollow {
    fn anchor_of(tuning: &MoveObjectTuning, grip: [f32; 3], back: [f32; 3]) -> [f32; 3] {
        [grip[0] + back[0] * tuning.anchor_reach, 0.0, grip[2] + back[2] * tuning.anchor_reach]
    }

    /// Grab: +416 = body position (82D442D0, Skeleton+15872), +624 = 0
    /// (82D43D70 reset), +608 = anchor of the new grip (82D444A0 begin).
    /// `back` is the latched frame row +368: horizontal, from the edge
    /// toward the skater.
    pub fn begin(tuning: &MoveObjectTuning, body: [f32; 3], grip: [f32; 3], back: [f32; 3]) -> Self {
        Self { point: body, anchor: Self::anchor_of(tuning, grip, back), velocity: [0.0; 3] }
    }

    /// 82D46610 (before the object command, so `back` is the previous
    /// tick's latch): v = keep v + new (anchor - previous anchor) / tick.
    pub fn update_anchor(&mut self, tuning: &MoveObjectTuning, grip: [f32; 3], back: [f32; 3]) {
        let anchor = Self::anchor_of(tuning, grip, back);
        let inv = 1.0 / tuning.tick;
        self.velocity = std::array::from_fn(|i| {
            tuning.anchor_velocity_keep * self.velocity[i] + tuning.anchor_velocity_new * (anchor[i] - self.anchor[i]) * inv
        });
        self.anchor = anchor;
    }

    /// The follow step after the object command (82D44A10 0x82D45110..):
    /// target = edge point (+192) + `follow_reach` x back (+368, this tick's
    /// latch), y = skater root height + `follow_height`; step = v tick +
    /// clamp_length(target - point - v tick, `follow_step`) (82BD41B0);
    /// point += step. Returns the new point.
    pub fn step(&mut self, tuning: &MoveObjectTuning, edge_point: [f32; 3], back: [f32; 3], root_y: f32) -> [f32; 3] {
        let target = [
            edge_point[0] + back[0] * tuning.follow_reach,
            root_y + tuning.follow_height,
            edge_point[2] + back[2] * tuning.follow_reach,
        ];
        let desired: [f32; 3] = std::array::from_fn(|i| target[i] - self.point[i]);
        let previous = self.velocity.map(|v| v * tuning.tick);
        let rest = clamp_length(std::array::from_fn(|i| desired[i] - previous[i]), tuning.follow_step);
        self.point = std::array::from_fn(|i| self.point[i] + previous[i] + rest[i]);
        self.point
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grabbing_lets_go_of_a_carried_board_and_hides_a_returning_one() {
        // 82D442D0 switch over SkateboardController+448.
        assert_eq!(board_on_grab(0), BoardOnGrab::LetGo);
        assert_eq!(board_on_grab(1), BoardOnGrab::LetGo);
        assert_eq!(board_on_grab(5), BoardOnGrab::LetGo);
        assert_eq!(board_on_grab(4), BoardOnGrab::Hide);
        assert_eq!(board_on_grab(2), BoardOnGrab::Keep);
        assert_eq!(board_on_grab(3), BoardOnGrab::Keep);
        assert_eq!(board_on_grab(6), BoardOnGrab::Keep, "above 5 skips the switch");
    }

    #[test]
    fn record_272_doubles_the_target_velocity() {
        // 82D45318: v x 2.0 (0x82060C50) when record+272 is non-zero.
        let t = MoveObjectTuning::default();
        let plain = command(&t, &mut MoveObjectController::default(), &input(1.0, 0.0, 0.0));
        let flagged = command(&t, &mut MoveObjectController::default(), &MoveObjectInput { record_272: true, ..input(1.0, 0.0, 0.0) });
        assert!((plain.target_velocity[2] - 3.0).abs() < 1e-6);
        assert!((flagged.target_velocity[2] - 6.0).abs() < 1e-6);
    }

    fn skater_frame(position: [f32; 3], forward: [f32; 3]) -> [[f32; 4]; 4] {
        let right = side_of(forward);
        [
            [right[0], right[1], right[2], 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [forward[0], forward[1], forward[2], 0.0],
            [position[0], position[1], position[2], 1.0],
        ]
    }

    #[test]
    fn hold_qualification_follows_the_retail_box_and_angles() {
        // A 1 m edge 0.65 m ahead at 0.5 m height, approach vector toward the
        // skater; grab box GrabBoxSizeGrabbing (0.9, 0.8, 1.01) at
        // GrabBoxOffset (0, 1, 0.1): z 0.1..2.12, y 0.2..1.8, x -0.9..0.9.
        let t = MoveObjectTuning::default();
        let r = edge_record(7, [-0.5, 0.5, 0.65], [0.5, 0.5, 0.65], [0.0, 0.0, -1.0]).unwrap();
        let hold = |position: [f32; 3], forward: [f32; 3]| {
            still_holds(&t, &r, [position[0], position[1] + 0.9, position[2]], 0.5, skater_frame(position, forward))
        };
        assert!(hold([0.0; 3], [0.0, 0.0, 1.0]), "facing the edge inside the box");
        // Grip 3.65 m ahead: beyond the box's far end (2.12 m).
        assert!(!hold([0.0, 0.0, -3.0], [0.0, 0.0, 1.0]), "too far behind");
        // Facing along the edge: |dot(forward, edge)| = 1, not < 0.8.
        assert!(!hold([0.0; 3], [1.0, 0.0, 0.0]), "facing along the edge");
        // 1.5 m ahead (still in the box) but past the edge: the reach points
        // away from -approach (180 deg > 80 deg).
        assert!(!hold([0.0, 0.0, 1.5], [0.0, 0.0, -1.0]), "on the far side");
        // 30 deg off the edge normal: within the 80 deg approach limit.
        let (s, c) = 30f32.to_radians().sin_cos();
        assert!(hold([0.0; 3], [s, 0.0, c]), "turned 30 deg");
    }

    #[test]
    fn skater_follow_steps_at_most_0_1_m_beyond_the_anchor_velocity() {
        let t = MoveObjectTuning::default();
        let back = [0.0, 0.0, -1.0];
        let grip = [0.0, 0.5, 0.0];
        let mut f = SkaterFollow::begin(&t, [0.0, 0.9, -1.0], grip, back);
        assert_eq!(f.anchor, [0.0, 0.0, -0.7]);
        // Still prop: velocity stays 0, the point moves 0.1 m per tick toward
        // edge - 0.65 back at root + 0.72 = (0, 0.72, -0.65).
        f.update_anchor(&t, grip, back);
        assert_eq!(f.velocity, [0.0; 3]);
        let before = f.point;
        let after = f.step(&t, grip, back, 0.0);
        let moved = ((after[0] - before[0]).powi(2) + (after[1] - before[1]).powi(2) + (after[2] - before[2]).powi(2)).sqrt();
        assert!((moved - 0.1).abs() < 1e-6, "first step {moved}");
        for _ in 0..3 {
            f.update_anchor(&t, grip, back);
            f.step(&t, grip, back, 0.0);
        }
        // Distance was 0.3936 m: four ticks reach the target exactly.
        let target = [0.0, 0.72, -0.65];
        assert!(f.point.iter().zip(target).all(|(a, b)| (a - b).abs() < 1e-6), "{:?}", f.point);
        // Prop moving at 3 m/s along +Z: anchor velocity 0.85 v + 0.15 x 3
        // converges to 3 m/s and the point tracks the moving target.
        let mut z = 0.0;
        for _ in 0..600 {
            z += 3.0 * t.tick;
            let g = [0.0, 0.5, z];
            f.update_anchor(&t, g, back);
            f.step(&t, g, back, 0.0);
        }
        assert!((f.velocity[2] - 3.0).abs() < 1e-3, "anchor velocity {}", f.velocity[2]);
        assert!((f.point[2] - (z - 0.65)).abs() < 1e-3, "tracks {} vs {}", f.point[2], z - 0.65);
    }

    fn input(mz: f32, mx: f32, rot: f32) -> MoveObjectInput {
        MoveObjectInput {
            move_z: mz,
            move_x: mx,
            move_rotation: rot,
            forward: [0.0, 0.0, 1.0],
            grip: [0.0, 0.0, 0.0],
            center: [0.0, 0.0, 0.5],
            velocity: [0.0; 3],
            heading: 0.0,
            mass: 50.0,
            yaw_inertia: 5.0,
            contact_normal: [0.0; 3],
            record_272: false,
        }
    }

    #[test]
    fn stock_curves_match_the_research_table() {
        let t = MoveObjectTuning::default();
        assert_eq!(t.speed_scale(50.0), 1.0);
        assert!((t.speed_scale(200.0) - 0.51).abs() < 1e-6);
        assert!((t.speed_scale(500.0) - 0.51).abs() < 1e-6, "clamped above the range");
        assert_eq!(t.yaw_gain(1.0), 2.0);
        assert!((t.lever_rotation.evaluate(2.0) - 1.0).abs() < 1e-6);
        assert_eq!(t.lever_rotation.evaluate(0.1), 0.0);
    }

    #[test]
    fn centre_push_gives_no_yaw_and_goes_forward() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        let c = command(&t, &mut s, &input(1.0, 0.0, 0.0));
        assert_eq!(c.lever, 0.0);
        assert_eq!(c.yaw_target, 0.0);
        assert_eq!(c.target_velocity, [0.0, 0.0, 3.0]);
        assert!(c.linear[2] > 0.0 && c.linear[0] == 0.0 && c.linear[1] == 0.0);
    }

    #[test]
    fn off_centre_push_turns_the_physical_way() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        // Grip 1.6 m right of the centre (centre to the skater's left).
        let mut i = input(1.0, 0.0, 0.0);
        i.center = [-1.6, 0.0, 0.5];
        let c = command(&t, &mut s, &i);
        // lever = dot(-edge, centre - grip) with edge = side_of(forward) = +X.
        assert!((c.lever - 1.6).abs() < 1e-6);
        assert!(c.rotation_demand > 0.5, "{c:?}");
        // The push at the grip (r = (1.6, 0, -0.5), F along +Z) has torque
        // r x F about +Y = -1.6 F: the yaw target has the same sign.
        assert!(c.yaw_target < 0.0, "off-centre push must turn the object like its torque: {c:?}");
        assert!(c.yaw < 0.0);
        // Positive OB_ObjectMvRot: w = -(curve x rot x gain) < 0 (82D45318).
        let mut s = MoveObjectController::default();
        let c = command(&t, &mut s, &input(0.0, 0.0, 1.0));
        assert!(c.yaw_target < 0.0 && c.yaw < 0.0, "{c:?}");
    }

    #[test]
    fn linear_controller_step_response_matches_hand_computation() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        let i = input(1.0, 0.0, 0.0); // target 3 m/s, object still
        // Tick 1: e = 3, out = 20*3 + 40*3 = 180 -> clamp 20 -> slew 4; the
        // sent 4 is written back over the output (anti-windup, 82D45318).
        let c = command(&t, &mut s, &i);
        assert!((c.linear[2] - 4.0).abs() < 1e-5);
        assert!((s.linear[2].output - 4.0).abs() < 1e-5);
        // Tick 2: e = 3, out = 4 + 60 + 0 = 64 -> clamp 20 -> slew 8.
        let c = command(&t, &mut s, &i);
        assert!((c.linear[2] - 8.0).abs() < 1e-5);
        assert!((s.linear[2].output - 8.0).abs() < 1e-5);
        // Ticks 3..: reaches the clamp 20 and stays; the output never winds past it.
        for _ in 0..5 {
            command(&t, &mut s, &i);
        }
        assert!((s.sent[2] - 20.0).abs() < 1e-5);
        assert!((s.linear[2].output - 20.0).abs() < 1e-5);
        // Object at the target speed: error 0, derivative 40 * -3: out = 20 - 120
        // = -100 -> clamp -20 -> slew 20 - 4 = 16, written back.
        let mut at_speed = i;
        at_speed.velocity = [0.0, 0.0, 3.0];
        let c = command(&t, &mut s, &at_speed);
        assert!((c.linear[2] - 16.0).abs() < 1e-4);
        assert!((s.linear[2].output - 16.0).abs() < 1e-4);
    }

    /// The yaw output is clamped and written back too (+1104, 82D45318).
    #[test]
    fn yaw_controller_output_is_clamped_and_written_back() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        let c = command(&t, &mut s, &input(0.0, 0.0, 1.0));
        assert!((c.yaw.abs() - t.yaw_clamp).abs() < 1e-5, "{c:?}");
        assert_eq!(s.yaw.output, c.yaw);
    }

    #[test]
    fn blocking_normal_removes_the_into_wall_part() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        let mut i = input(1.0, 1.0, 0.0);
        i.contact_normal = [0.0, 0.0, -1.0]; // wall ahead
        let c = command(&t, &mut s, &i);
        assert!(c.blocked);
        assert!(c.target_velocity[2].abs() < 1e-6, "{c:?}");
        assert!(c.target_velocity[0] > 0.0, "along the wall kept: {c:?}");
        // Decays away once free.
        i.contact_normal = [0.0; 3];
        for _ in 0..200 {
            command(&t, &mut s, &i);
        }
        assert!(!command(&t, &mut s, &i).blocked);
    }

    #[test]
    fn heading_latch_holds_small_drift_and_relatches_large() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        let mut i = input(1.0, 0.0, 0.0);
        command(&t, &mut s, &i);
        i.heading = 0.05;
        i.forward = [0.05f32.sin(), 0.0, 0.05f32.cos()];
        let c = command(&t, &mut s, &i);
        assert!((c.drift - 0.05).abs() < 1e-6);
        assert_eq!(s.latched_forward, [0.0, 0.0, 1.0], "small drift keeps the latched frame");
        // The facing moved +0.05 rad in one tick: measured rate +3 rad/s
        // against a zero target, so the yaw command pushes back.
        assert!((c.yaw_rate - 3.0).abs() < 1e-4, "{c:?}");
        assert!(c.yaw < 0.0, "the turn is braked");
        i.heading = 0.3;
        let c = command(&t, &mut s, &i);
        assert_eq!(c.drift, 0.0);
        assert_eq!(s.latched_yaw, 0.3);
    }

    /// Yaw error = w - 60 x (facing change since the previous tick)
    /// (82D45318, 0x82D45BC8..0x82D45CD0), hand-computed.
    #[test]
    fn yaw_error_is_target_minus_measured_rate() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        // Tick 1: no previous facing (+384 zero) -> rate 0. w = -(1 x 1 x 2) = -2:
        // out = 20 x -2 + 40 x -2 = -120 -> clamp -6.
        let c = command(&t, &mut s, &input(0.0, 0.0, 1.0));
        assert_eq!(c.yaw_rate, 0.0);
        assert!((c.yaw_target + 2.0).abs() < 1e-6 && (c.yaw + 6.0).abs() < 1e-6, "{c:?}");
        // Tick 2: the facing turned -2/60 rad (the target rate): error 0,
        // derivative 40 x (0 - -2) = 80: out = -6 + 80 = 74 -> clamp 6.
        let mut i = input(0.0, 0.0, 1.0);
        i.heading = -2.0 / 60.0;
        let c = command(&t, &mut s, &i);
        assert!((c.yaw_rate + 2.0).abs() < 1e-4, "{c:?}");
        assert!((c.yaw - 6.0).abs() < 1e-5, "{c:?}");
        // The wrap: a facing step across +-pi is a small rate, not 2 pi x 60.
        let mut s = MoveObjectController { facing_yaw: 3.13, facing_valid: true, ..Default::default() };
        let mut i = input(0.0, 0.0, 0.0);
        i.heading = -3.13;
        let c = command(&t, &mut s, &i);
        assert!((c.yaw_rate - (std::f32::consts::TAU - 6.26) * 60.0).abs() < 1e-2, "{c:?}");
    }

    /// The retail loop on a free yaw body (command applied as an angular
    /// acceleration for one 60 Hz step, slot 9 / 82D9CCF0) settles at the
    /// target rate: the controller output accumulates (integral action), so
    /// the steady error is zero, and the 6 rad/s^2 clamp only limits the
    /// spin-up (0.1 rad/s per tick).
    #[test]
    fn yaw_rate_feedback_settles_at_the_target_rate() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        let mut i = input(0.0, 0.0, 1.0);
        let (mut rate, mut peak) = (0.0f32, 0.0f32);
        for tick in 0..180 {
            let c = command(&t, &mut s, &i);
            rate += c.yaw / 60.0;
            i.heading = wrap(i.heading + rate / 60.0);
            peak = peak.max(rate.abs());
            if tick < 10 {
                assert!((rate + 0.1 * (tick + 1) as f32).abs() < 1e-4, "tick {tick}: {rate}");
            }
        }
        assert!((rate + 2.0).abs() < 1e-3, "settled at {rate}, target -2");
        assert!(peak < 2.02, "overshoot {peak}");
    }

    #[test]
    fn controller_state_round_trips_through_the_flat_form() {
        let t = MoveObjectTuning::default();
        let mut s = MoveObjectController::default();
        let mut i = input(0.7, -0.3, 0.2);
        i.contact_normal = [0.1, 0.0, -0.9];
        for _ in 0..7 {
            command(&t, &mut s, &i);
        }
        assert_eq!(MoveObjectController::from_array(s.to_array()), s);
        // Determinism: the same inputs from the restored state give the same command.
        let mut copy = MoveObjectController::from_array(s.to_array());
        assert_eq!(command(&t, &mut s, &i), command(&t, &mut copy, &i));
    }

    #[test]
    fn bad_mod_values_fall_back() {
        let d = MoveObjectTuning::default();
        let bad = MoveObjectTuning { push_speed: f32::NAN, yaw_clamp: -1.0, slew_per_tick: f32::INFINITY, ..d };
        let s = bad.sanitized(&d);
        assert_eq!(s, d);
    }
}
