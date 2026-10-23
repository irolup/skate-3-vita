//! Carrying and placing dynamic props while offboard (Phases 3-4 glue).
//!
//! Buttons (raw controller flag bits, see `CarryButtons`): holding the retail
//! GrabWorld button (RB) grabs the nearest prop in reach and keeps it while
//! held; releasing drops it. **B** toggles placement mode while carrying, and
//! releasing the grab button in placement mode confirms the ghost pose.
//!
//! The grab used to be a rising edge of **A**, but A is the retail sprint
//! button: the derived controller's held timer for action 80 (slot 20) is what
//! publishes `OB_Sprint` (8259AA8C, `offboard_intentions.rs`), so every sprint
//! press near a prop grabbed it and dragged it along ("props magnetize to the
//! player as they run by"). Retail gates the offboard grab-object decision
//! (82D324B0, `biped_ground/grab.rs`) on Processed2476 bit 22, GrabWorld,
//! which the input listener emits while raw flag bit 28 (action 73, RB) is
//! held (8259AF68): a held button, not a toggle.
//!
//! Carry is retail Move Object (state 502, research note
//! move-object-retail.md): the held prop stays a normal dynamic body (gravity,
//! contacts, tipping) and every fixed tick receives one bounded command, a
//! horizontal linear term plus a yaw term from two velocity-tracking
//! controllers (82D45318, `skate_core::player::offboard::move_object`), through
//! `PropDynamics::apply_move_command`. The prop leads and the skater follows
//! its grab edge (82D45D30): [`PropCarry::skater_target`] is read by
//! `biped_ground`. The held prop neither receives skater pushes nor pushes the
//! skater back: `PropDynamics::set_held` exempts it from volume pushes and
//! its collision-layer triangles stay parked at `HELD_PARK` until the drop
//! rebakes them.
//!
//! Placement mode (Phase 4): the prop keeps following a target pose relative
//! to the carrier, frozen mid-air by the per-tick velocity overwrite, while
//! the right stick adjusts distance (Y) and yaw (X) and DPad up/down adjusts
//! height. Releasing the grab button confirms: the prop is dropped at the
//! ghost pose and the layout sidecar is rewritten. B cancels back to plain
//! carry. Yaw applies a direct
//! orientation snap; position still moves by velocity so contacts and the
//! rebaked triangle layer keep working.
//!
//! Layout persistence (`prop_layout.rs`): confirming a placement records
//! `id → (origin, basis)` and saves `settings/prop-layouts/<map>.json` next
//! to the asset root (same convention as `settings/gameplay.json`). Loading
//! a map teleports saved bodies to their stored poses before the first sync.
//!
//! Restrictions: grabbing requires `BipedGround`; a single prop at a time;
//! the grab is kept while the held grab record qualifies (retail 82D44A10 ->
//! CanGrabSpline 82E08EE8 at the grip, see `still_holds`), and dropped when it
//! stops qualifying or when leaving the on-foot states
//! (`BipedGround`/`OffBoardPushing`; never saves). While held, the retail
//! grab-object byte (`OffBoard304`, published in `player_state/publication.rs`)
//! keeps the selector in `OffBoardPushing` and the MotionGraph in
//! MovingObjectNew, so carrying must not treat state 502 as leaving the ground. Drop keeps the current velocity, so releasing while moving throws
//! gently; re-sleep is the natural cool-down.
//!
//! Locomotion while holding: retail's producer 8259C4B0 turns the raw left
//! stick into OB_ObjectMvX/Z and the right stick X into OB_ObjectMvRot. The
//! command reads them in the prop's latched grab-edge frame: Z pushes / pulls
//! along the edge normal, X slides along the edge, an off-centre push turns
//! the prop through the lever-arm curves, the right stick turns it.
//!
//! Multiplayer: a `PropCarry` is one player slot's state; its Move Object
//! controller is plain data (`controller_snapshot`), the command is a pure
//! function per fixed tick. Prop bodies and layouts are host-local; a host
//! would replicate held id, body poses / velocities and layout writes. No
//! transport is implemented.
use skate_core::{
    math::Vector3,
    player::offboard::move_object::{MoveObjectCommand, MoveObjectController, MoveObjectInput, MoveObjectTuning, SkaterFollow},
    player::state::PhysicalStateId,
};
use skate_core::player::offboard::grab_scene::Record;
use std::collections::BTreeMap;

use super::prop_dynamics::PropDynamics;

//// Pickup reach from the carrier's root, measured to the prop's surface.
/// Omnidirectional: retail grabbing does not require facing the prop.
const GRAB_RADIUS: f32 = 2.0;
/// Placement follow cap; above this the prop lags instead of snapping.
const MAX_CARRY_SPEED: f32 = 6.0;
/// Placement adjust rates and clamps.
const PLACE_YAW_RATE: f32 = 2.5;
const PLACE_DISTANCE_RATE: f32 = 2.0;
const PLACE_HEIGHT_RATE: f32 = 1.5;
const PLACE_DISTANCE: std::ops::Range<f32> = 0.3..4.0;
const PLACE_HEIGHT: std::ops::Range<f32> = 0.0..2.5;

/// Move Object (state 502) tuning in effect: the retail command tuning
/// (attribute class 3EDA5B140604613D, loaded from the stock collection) plus
/// the engine values that stand in for still-undecoded retail parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CarryLocomotion {
    /// Retail 82D45318 tuning (speeds, curves, controllers, clamps).
    pub move_object: MoveObjectTuning,
    /// Optional constant yaw gain replacing the inertia curve
    /// (EABFCC79873A2859) for a mod; `None` = retail curve.
    pub turn_rate: Option<f32>,
    /// Optional engine rule for a mod: m the skater may fall behind its
    /// follow point (beyond the closest it got) before the prop is let go;
    /// 0 = off (retail: only the record qualification, `still_holds`).
    pub let_go_distance: f32,
    /// Half the hand spread kept from the edge ends when choosing the grip
    /// point (82D444A0 clamps the grip to [d/2, length - d/2] with d from the
    /// hand bones 3 / 7). NOT RETAIL YET: engine value.
    pub hand_half_spread: f32,
    /// Entering Move Object drops a carried board (retail 82D442D0 lets go
    /// of it, 82D75440; true). A mod may keep the board in hand (false).
    pub drop_board: bool,
}

impl Default for CarryLocomotion {
    fn default() -> Self {
        Self::from_tuning(MoveObjectTuning::default())
    }
}

impl CarryLocomotion {
    /// Defaults over a retail tuning loaded from the setup data.
    pub(crate) fn from_tuning(move_object: MoveObjectTuning) -> Self {
        Self { move_object, turn_rate: None, let_go_distance: 0.0, hand_half_spread: 0.25, drop_board: true }
    }

    /// Non-finite or negative values fall back to `base` per field.
    pub(crate) fn sanitized(self, base: &Self) -> Self {
        let ok = |v: f32, d: f32| if v.is_finite() && v >= 0.0 { v } else { d };
        Self {
            move_object: self.move_object.sanitized(&base.move_object),
            turn_rate: self.turn_rate.filter(|v| v.is_finite() && *v >= 0.0),
            let_go_distance: ok(self.let_go_distance, base.let_go_distance),
            hand_half_spread: ok(self.hand_half_spread, base.hand_half_spread),
            drop_board: self.drop_board,
        }
    }

    /// The command tuning with the mod's constant turn rate folded in.
    fn command_tuning(&self) -> MoveObjectTuning {
        let mut t = self.move_object;
        if let Some(rate) = self.turn_rate {
            t.inertia_yaw_gain = skate_core::point_graph::PointGraph { x: [0.0; 8], y: [rate; 8] };
        }
        t
    }
}

/// Mod overrides of [`CarryLocomotion`] (`sdk.world.set_tuning('carry', ...)`):
/// every field optional, applied over the tuning loaded from the setup data,
/// so disabling the mod restores the retail values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LocomotionOverrides {
    pub push_speed: Option<f32>,
    pub pull_speed: Option<f32>,
    pub side_speed: Option<f32>,
    pub turn_rate: Option<f32>,
    pub grip_reach: Option<f32>,
    pub linear_clamp: Option<f32>,
    pub yaw_clamp: Option<f32>,
    pub relatch: Option<f32>,
    pub slew_per_tick: Option<f32>,
    pub yaw_rate_feedback: Option<f32>,
    pub linear_controller: Option<[f32; 4]>,
    pub yaw_controller: Option<[f32; 4]>,
    pub lever_rotation: Option<[[f32; 8]; 2]>,
    pub lever_yaw: Option<[[f32; 8]; 2]>,
    pub mass_speed: Option<[[f32; 8]; 2]>,
    pub inertia_yaw_gain: Option<[[f32; 8]; 2]>,
    pub let_go_distance: Option<f32>,
    pub drop_board: Option<bool>,
    pub follow_step: Option<f32>,
    pub hold_angle_limit: Option<f32>,
    pub hold_max_angle_to_horizontal: Option<f32>,
    pub hold_box_extents: Option<[f32; 3]>,
    pub record_272_speed_scale: Option<f32>,
    pub grab_end_exclusion: Option<f32>,
    pub hand_ik_enter: Option<f32>,
    pub hand_ik_curve: Option<[[f32; 8]; 2]>,
    pub hand_ik_rate: Option<f32>,
    pub hand_ik_reach: Option<f32>,
    pub root_yaw_gain: Option<[[f32; 8]; 2]>,
}

impl LocomotionOverrides {
    pub(crate) fn apply(&self, base: CarryLocomotion) -> CarryLocomotion {
        let mut l = base;
        let m = &mut l.move_object;
        let gains = |g: [f32; 4]| skate_core::player::offboard::move_object::ControllerGains {
            proportional: g[0],
            filtered: g[1],
            derivative: g[2],
            filter: g[3],
        };
        let curve = |c: [[f32; 8]; 2]| skate_core::point_graph::PointGraph { x: c[0], y: c[1] };
        if let Some(v) = self.push_speed { m.push_speed = v; }
        if let Some(v) = self.pull_speed { m.pull_speed = v; }
        if let Some(v) = self.side_speed { m.side_speed = v; }
        if let Some(v) = self.linear_clamp { m.linear_clamp = v; }
        if let Some(v) = self.yaw_clamp { m.yaw_clamp = v; }
        if let Some(v) = self.relatch { m.relatch = v; }
        if let Some(v) = self.slew_per_tick { m.slew_per_tick = v; }
        if let Some(v) = self.yaw_rate_feedback { m.yaw_rate_feedback = v; }
        if let Some(v) = self.linear_controller { m.linear_controller = gains(v); }
        if let Some(v) = self.yaw_controller { m.yaw_controller = gains(v); }
        if let Some(v) = self.lever_rotation { m.lever_rotation = curve(v); }
        if let Some(v) = self.lever_yaw { m.lever_yaw = curve(v); }
        if let Some(v) = self.mass_speed { m.mass_speed = curve(v); }
        if let Some(v) = self.inertia_yaw_gain { m.inertia_yaw_gain = curve(v); }
        if let Some(v) = self.root_yaw_gain { m.root_yaw_gain = curve(v); }
        // `grip_reach` (the old engine reach) now sets the retail follow
        // reach (0.65 m, 82D44A10) between the edge point and the body.
        if let Some(v) = self.grip_reach { m.follow_reach = v; }
        if let Some(v) = self.let_go_distance { l.let_go_distance = v; }
        if let Some(v) = self.drop_board { l.drop_board = v; }
        let m = &mut l.move_object;
        if let Some(v) = self.follow_step { m.follow_step = v; }
        if let Some(v) = self.hold_angle_limit { m.hold_angle_limit = v; }
        if let Some(v) = self.hold_max_angle_to_horizontal { m.hold_max_angle_to_horizontal = v; }
        if let Some(v) = self.hold_box_extents { m.hold_box_extents = v; }
        if let Some(v) = self.record_272_speed_scale { m.record_272_speed_scale = v; }
        if let Some(v) = self.grab_end_exclusion { m.grab_end_exclusion = v; }
        if let Some(v) = self.hand_ik_enter { m.hand_ik_enter = v; }
        if let Some(v) = self.hand_ik_curve { m.hand_ik_curve = curve(v); }
        if let Some(v) = self.hand_ik_rate { m.hand_ik_rate = v; }
        if let Some(v) = self.hand_ik_reach { m.hand_ik_reach = v; }
        l.turn_rate = self.turn_rate.or(base.turn_rate);
        l.sanitized(&base)
    }
}

/// The interim grab record of a held prop: which vertical box face is held
/// (local axis index and side) and the grip position along its edge, both in
/// the prop's own frame so the grip turns with the prop. NOT RETAIL YET:
/// the stand-in for props without authored grab splines (and for every prop
/// while `SKATE_PROP_GRAB` is off); props with splines carry by their
/// authored record ([`PropCarry::frame_for`]). The face toward the skater at
/// grab time is the grab edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GrabEdge {
    /// Local box axis of the face normal (0 or 2) and its sign.
    pub axis: usize,
    pub side: f32,
    /// Grip position along the edge (local axis `2 - axis`), metres from the centre.
    pub grip: f32,
}

/// The held prop's grab frame this tick (world space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GrabFrame {
    /// Grip point on the grab face (centre height).
    pub grip: Vector3,
    /// Horizontal unit normal from the skater into the prop.
    pub forward: Vector3,
    /// Where the skater's root belongs: the root moved by this tick's step
    /// of the retail follow point +416; `grip` until the first command.
    pub skater: Vector3,
    /// The grab edge as a straight segment (start, end) and the grip's arc
    /// distance from its start: the interim grab record's geometry.
    pub ends: [Vector3; 2],
    pub grip_distance: f32,
}

fn grab_frame(body: &HeldBody, edge: GrabEdge) -> Option<GrabFrame> {
    let axis = body.basis.columns[edge.axis];
    let along = body.basis.columns[2 - edge.axis];
    let half = [body.half_extents.x, body.half_extents.y, body.half_extents.z];
    let out = flat_unit(Vector3::new(axis[0] * edge.side, 0.0, axis[2] * edge.side))?;
    let edge_dir = flat_unit(Vector3::new(along[0], 0.0, along[2]))?;
    let face = add(body.center, scale(out, half[edge.axis] * flat_len(Vector3::new(axis[0], 0.0, axis[2]))));
    let grip = add(face, scale(edge_dir, edge.grip));
    let forward = scale(out, -1.0);
    let half_along = half[2 - edge.axis] * flat_len(Vector3::new(along[0], 0.0, along[2]));
    // The interim record lies on the face's top edge (NOT RETAIL YET: a
    // prop's authored grab splines, DMO physics definition +136, are not
    // loaded); the box's vertical half height over its centre.
    let top = (0..3).map(|i| (half[i] * body.basis.columns[i][1]).abs()).sum::<f32>();
    let lift = Vector3::new(0.0, top, 0.0);
    let ends = [add(add(face, scale(edge_dir, -half_along)), lift), add(add(face, scale(edge_dir, half_along)), lift)];
    Some(GrabFrame { grip, forward, skater: grip, ends, grip_distance: half_along + edge.grip })
}

/// The grab frame of an authored record at the grip (82D45D30): grip point,
/// facing minus the flattened approach vector, the record's ends.
fn authored_frame(record: &Record, held: &skate_core::player::offboard::move_object::HeldGrip) -> Option<GrabFrame> {
    let at = skate_core::player::offboard::move_object::record_frame(record, held.grip);
    let v = |a: [f32; 3]| Vector3::new(a[0], a[1], a[2]);
    let grip = v(at.point);
    Some(GrabFrame { grip, forward: flat_unit(scale(v(at.approach), -1.0))?, skater: grip, ends: [v(at.ends[0]), v(at.ends[1])], grip_distance: held.grip })
}

/// The authored grab records of prop `id` from its current pose; `None`
/// without grab splines or when `enabled` is false.
pub(crate) fn authored_records(dynamics: &PropDynamics, id: u32, enabled: bool) -> Option<Vec<Record>> {
    if !enabled {
        return None;
    }
    let object = dynamics.grab_object(id)?;
    let records: Vec<_> = object.splines.iter().filter_map(|s| object.record(s).ok()).collect();
    (!records.is_empty()).then_some(records)
}

/// The face of the held box toward `point` and the grip along it, clamped
/// `hand_half_spread` inside the edge ends (82D444A0's grip clamp).
fn choose_edge(body: &HeldBody, point: Vector3, hand_half_spread: f32) -> GrabEdge {
    let to = sub(point, body.center);
    let halves = [body.half_extents.x, body.half_extents.y, body.half_extents.z];
    let mut best = (0usize, 1.0f32, f32::NEG_INFINITY);
    for axis in [0usize, 2] {
        let a = body.basis.columns[axis];
        let Some(n) = flat_unit(Vector3::new(a[0], 0.0, a[2])) else { continue };
        for side in [1.0f32, -1.0] {
            // Distance of the point outside this face's plane (not the raw
            // projection, which picks the end face of a long bench for a
            // skater standing behind its long side).
            let score = side * dot(n, to) - halves[axis] * flat_len(Vector3::new(a[0], 0.0, a[2]));
            if score > best.2 {
                best = (axis, side, score);
            }
        }
    }
    let along = body.basis.columns[2 - best.0];
    let half = [body.half_extents.x, body.half_extents.y, body.half_extents.z][2 - best.0];
    let edge_dir = flat_unit(Vector3::new(along[0], 0.0, along[2])).unwrap_or(Vector3::new(1.0, 0.0, 0.0));
    let limit = (half - hand_half_spread).max(0.0);
    GrabEdge { axis: best.0, side: best.1, grip: dot(to, edge_dir).clamp(-limit, limit) }
}

// Which raw controller flag bits drive carrying. The bits index the packed
/// button word of `DerivedControllerInput` (word 6 previous, word 13 current;
/// bit = 101 - action for actions 74..81, bits 28..31 for actions 73..66).
/// Defaults are the retail buttons; a host setting or mod may rebind them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CarryButtons {
    /// Held to grab and carry: bit 28, action 73, RB (retail GrabWorld).
    pub grab_bit: u32,
    /// Rising edge toggles placement: bit 20, action 81, B.
    pub placement_bit: u32,
}

impl Default for CarryButtons {
    fn default() -> Self {
        Self {
            grab_bit: 28,
            placement_bit: 20,
        }
    }
}

/// One tick of carry/placement input, sampled in `frame.rs`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Tick {
    /// Grab button held (level): grabs, keeps the carry, release drops or
    /// confirms placement.
    pub grab: bool,
    /// B rising edge: enter placement or cancel back to carry.
    pub placement: bool,
    /// Right stick X/Y and DPad up-down, already scaled to [-1, 1].
    pub yaw_axis: f32,
    pub distance_axis: f32,
    pub height_axis: f32,
    /// OB_ObjectMvX / OB_ObjectMvZ / OB_ObjectMvRot (8259C4B0) this tick.
    pub object_move: [f32; 3],
    /// Skater's own contact normal (retail Player+16304) for the blocking
    /// normal; zero when free. NOT WIRED YET: the engine passes zero.
    pub contact_normal: Vector3,
}

impl Tick {
    /// Buttons from the derived controller words (`DerivedControllerInput::words`).
    pub(crate) fn from_controller(
        words: &[u32; 26],
        buttons: CarryButtons,
        yaw_axis: f32,
        distance_axis: f32,
        height_axis: f32,
    ) -> Self {
        let held = |bit: u32| bit < 32 && words[13] & (1 << bit) != 0;
        let was_held = |bit: u32| bit < 32 && words[6] & (1 << bit) != 0;
        Self {
            grab: held(buttons.grab_bit),
            placement: held(buttons.placement_bit) && !was_held(buttons.placement_bit),
            yaw_axis,
            distance_axis,
            height_axis,
            object_move: [0.0; 3],
            contact_normal: Vector3::ZERO,
        }
    }
}

//// Per-tick carrier observation from the skater runtime.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Carrier {
    pub state: PhysicalStateId,
    pub position: Vector3,
    /// Horizontal facing direction, normalized.
    pub forward: Vector3,
    pub time_step: f32,
    /// Retail skater observations for Move Object; `None` (host tests, the
    /// HUD) skips the record qualification and uses `position` as body.
    pub skeleton: Option<CarrierSkeleton>,
    /// The Ground grab decision this tick (82D324B0, `biped_ground::grab`): `Some(Some(descriptor))` when a validated
    /// record qualified (state +144, descriptor +152 / +156), `Some(None)` when none did or in another state. The game
    /// always passes one: a carry then starts only from the Move Object enter's bind of the published record
    /// (`bind_published`, b233). `None` (host tests, the HUD) has no decision and keeps the interim pick (nearest prop
    /// within `grab_range`, its nearest record).
    pub ground_grab: Option<Option<(u32, u32)>>,
}

/// The skater inputs retail Move Object reads, from the skater runtime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CarrierSkeleton {
    /// Player+192 effective frame (rows right, up, forward, position).
    pub frame: [[f32; 4]; 4],
    /// Bone 23 world position (state +272, 82BE3220 in 82D444A0).
    pub reference: Vector3,
    /// Body position Skeleton+15872 (state +416 at the grab, 82D442D0).
    pub body: Vector3,
    /// Distance between the hand bones 3 and 7 (82D45D30 hand points).
    pub hand_span: f32,
    /// Collision flag Player+2484 bit 26 (`flag_215`): drives the rebind timer +1176 (82D44A10).
    pub collision_flag: bool,
    /// Time in the current player state, s (Player+2664; the 82D46610 hand IK gate).
    pub state_time: f32,
}

impl Carrier {
    /// Rotation about +Y mapping (0,0,1) onto `forward`.
    fn facing_yaw(&self) -> f32 {
        f32::atan2(self.forward.x, self.forward.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Carry,
    /// Ghost pose offsets relative to the carrier's root and facing.
    Placement { distance: f32, height: f32, yaw: f32 },
}

/// The held prop as Move Object sees it (from `PropDynamics::held_body`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeldBody {
    pub center: Vector3,
    /// Box axes as rows (`columns[i]` = local axis i in world space).
    pub basis: skate_core::math::Basis3,
    pub half_extents: Vector3,
    pub velocity: Vector3,
    pub mass: f32,
    /// Inertia about the box's up axis (1 / inverse tensor y).
    pub yaw_inertia: f32,
    /// Record+272 for this prop type (82C4B960 from DMO data +312; per
    /// template `carry.by_template[...].record_272`, default false: the DMO
    /// type data is not extracted yet).
    pub record_272: bool,
}

impl HeldBody {
    /// Heading of the box's local +Z about world +Y, in the sense of the
    /// body's angular velocity (a positive yaw rate turns +Z toward +X and
    /// increases it; test `positive_yaw_rate_turns_local_z_toward_plus_x`),
    /// so the yaw-rate feedback of the yaw controller is negative as in 82D45318.
    fn heading(&self) -> f32 {
        let z = self.basis.columns[2];
        f32::atan2(z[0], z[2])
    }
}

/// One player slot's carry state (this whole struct is per slot; the local
/// player is slot 0). Authority: the prop simulation applies this slot's
/// command in one place ([`PropDynamics::apply_move_command`]) once per fixed
/// physics tick; nothing here reads a wall clock.
#[derive(Clone, Debug, Default)]
pub(crate) struct PropCarry {
    held: Option<u32>,
    mode: Mode,
    /// Confirmed placements this session, seeded from the layout sidecar so
    /// re-saving never drops props placed in earlier sessions.
    layout: BTreeMap<u32, super::prop_layout::PropPose>,
    layout_path: Option<std::path::PathBuf>,
    buttons: CarryButtons,
    /// Pickup reach (`GRAB_RADIUS` unless [`CarrySettings`] changes it).
    grab_range: Option<f32>,
    /// Move Object tuning loaded from the setup data (mod-free base).
    base: CarryLocomotion,
    /// Move Object tuning in effect (base + mod overrides).
    locomotion: CarryLocomotion,
    /// Retail controller state of this slot (serialisable plain data).
    controller: MoveObjectController,
    /// Interim grab record of the held prop (props without authored grab splines).
    edge: Option<GrabEdge>,
    /// Held authored record (descriptor, grip +1128, reversed bit) for props
    /// with grab splines (82D444A0, Move Object step 3).
    bound: Option<skate_core::player::offboard::move_object::HeldGrip>,
    /// Carry by authored records: `None` = `SKATE_PROP_GRAB=1` (tests set it).
    authored: Option<bool>,
    /// Rebind block state (+1176 timer, +1200 bit 0x02 latch) and the hand
    /// points of the held record (82D45D30), for authored records.
    rebind: skate_core::player::offboard::move_object::RebindState,
    rebind_tuning: skate_core::player::offboard::move_object::RebindTuning,
    hands: Option<[[f32; 3]; 2]>,
    /// Hand IK bit +1200 0x40 and weight +1132 (82D46610 / 82D45008) on the hand points.
    hand_ik: skate_core::player::offboard::move_object::HandIk,
    /// Grab frame of the held prop after this tick's command: where the
    /// skater is pulled to and which way it faces (read by `biped_ground`).
    frame: Option<GrabFrame>,
    /// Closest the skater got to its follow point (optional mod let-go rule).
    closest: f32,
    /// The grab button last tick (the press edge for `PROP_GRAB_MISS`).
    grab_down: bool,
    /// The published record the Move Object enter (82D442D0) passed its gate with, bound at the next update.
    pending_bind: Option<Record>,
    /// Retail skater follow state (+416 / +608 / +624); `None` until the
    /// first held tick.
    follow: Option<SkaterFollow>,
    /// The last stick input `CARRY_STICK` logged (tenths of OB_ObjectMvZ / MvX / MvRot), so it writes on change.
    stick_logged: Option<[i32; 3]>,
    /// Held steps since the grab (`None` until CARRY_WEIGHT is logged for this grab); CARRY_ORBIT every 15 while turning.
    held_steps: Option<u32>,
    /// Last command, for HELD_PROP.
    last: MoveObjectCommand,
    last_input: [f32; 3],
}

/// Carry settings a host setting or a mod (`sdk.world.set_tuning('carry', ...)`) changes:
/// the buttons, the pickup reach and Move Object overrides. `default()` = shipped values (mod
/// disable). Pushed into the live [`PropCarry`] before each physics tick, so a map load (new
/// `PropCarry`) keeps them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CarrySettings {
    pub buttons: CarryButtons,
    pub grab_range: f32,
    pub locomotion: LocomotionOverrides,
    /// How the Move Object command reaches the held prop (retail defaults;
    /// pushed into [`PropDynamics`], so a map load keeps them).
    pub move_rules: super::prop_dynamics::MoveCommandRules,
}

impl Default for CarrySettings {
    fn default() -> Self {
        Self {
            buttons: CarryButtons::default(),
            grab_range: GRAB_RADIUS,
            locomotion: LocomotionOverrides::default(),
            move_rules: Default::default(),
        }
    }
}

impl CarrySettings {
    /// Push into the live carry state (non-finite or non-positive reach = default).
    pub(crate) fn apply_to(&self, carry: &mut PropCarry) {
        carry.buttons = self.buttons;
        carry.grab_range = (self.grab_range.is_finite() && self.grab_range > 0.0 && self.grab_range != GRAB_RADIUS).then_some(self.grab_range);
        carry.locomotion = self.locomotion.apply(carry.base);
    }
}


impl Default for Mode {
    fn default() -> Self {
        Self::Carry
    }
}

impl PropCarry {
    pub fn held(&self) -> Option<u32> {
        self.held
    }

    pub(crate) fn buttons(&self) -> CarryButtons {
        self.buttons
    }

    /// Move Object tuning in effect.
    pub(crate) fn locomotion(&self) -> CarryLocomotion {
        self.locomotion
    }

    /// Tuning from the setup data before mod overrides.
    pub(crate) fn base_locomotion(&self) -> CarryLocomotion {
        self.base
    }

    /// Set the tuning loaded from the setup data (map load); mod overrides
    /// are re-applied on the next `apply_carry_settings`.
    pub(crate) fn set_base_tuning(&mut self, tuning: MoveObjectTuning) {
        self.base = CarryLocomotion::from_tuning(tuning);
        self.locomotion = self.base;
    }

    /// Where the skater belongs while holding (Move Object: the skater follows
    /// the prop's grab edge, 82D45D30), as (root position, facing).
    pub(crate) fn skater_target(&self) -> Option<(Vector3, Vector3)> {
        self.held?;
        self.frame.map(|f| (f.skater, f.forward))
    }

    /// Share of the angle to the grab edge the skater root turns this tick
    /// (82D46610 +1124: curve 023D73966B8669A5 at the time held); 1.0 (a
    /// snap) before the follow state exists.
    pub(crate) fn root_yaw_gain(&self) -> f32 {
        self.follow.map_or(1.0, |f| f.root_yaw_gain)
    }

    /// Snapshot of this slot's Move Object state: held id and the flat
    /// controller state (multiplayer-ready; no transport here).
    #[allow(dead_code)]
    pub(crate) fn controller_snapshot(&self) -> Option<(u32, [f32; skate_core::player::offboard::move_object::CONTROLLER_FLOATS])> {
        Some((self.held?, self.controller.to_array()))
    }

    /// Last command sent and the stick that produced it (HELD_PROP).
    #[allow(dead_code)]
    pub(crate) fn last_command(&self) -> Option<(MoveObjectCommand, [f32; 3])> {
        self.held.map(|_| (self.last, self.last_input))
    }

    /// Pickup reach in effect.
    pub(crate) fn grab_range(&self) -> f32 {
        self.grab_range.unwrap_or(GRAB_RADIUS)
    }

    /// Rebind the carry buttons (host setting or mod override).
    #[allow(dead_code)]
    pub(crate) fn set_buttons(&mut self, buttons: CarryButtons) {
        self.buttons = buttons;
    }

    /// Force carrying by authored records on or off (tests).
    #[cfg(test)]
    pub(crate) fn set_authored_records(&mut self, on: bool) {
        self.authored = Some(on);
    }

    /// The held authored record state (tests, diagnostics).
    #[allow(dead_code)]
    /// The hand IK targets (hand A = p+, hand B = p-; 82BD9728 / 82BD97D0) and weight, while the weight is above 0.
    pub(crate) fn hand_ik_targets(&self) -> Option<([[f32; 3]; 2], f32)> {
        let hands = self.hands?;
        (self.hand_ik.weight > 0.0).then_some((hands, self.hand_ik.weight))
    }

    pub(crate) fn held_grip(&self) -> Option<skate_core::player::offboard::move_object::HeldGrip> {
        self.bound
    }

    pub fn placing(&self) -> bool {
        matches!(self.mode, Mode::Placement { .. })
    }

    /// Session state after loading a map: saved poses plus the sidecar path
    /// (None keeps placement working without persistence).
    pub(crate) fn with_layout(
        layout: BTreeMap<u32, super::prop_layout::PropPose>,
        layout_path: Option<std::path::PathBuf>,
    ) -> Self {
        Self { layout, layout_path, ..Self::default() }
    }

    /// Drop saved placements (props reset to their spawn pose) and rewrite the sidecar.
    pub(crate) fn forget_layout(&mut self, ids: &[u32]) {
        let before = self.layout.len();
        for id in ids {
            self.layout.remove(id);
        }
        if self.layout.len() == before {
            return;
        }
        if let Some(path) = &self.layout_path {
            if let Err(error) = super::prop_layout::save(path, &self.layout) {
                warn!("SKATE_PROP_LAYOUT: {}: {error}", path.display());
            }
        }
    }

    /// Saved pose overrides, for load-time application.
    pub(crate) fn layout(&self) -> &BTreeMap<u32, super::prop_layout::PropPose> {
        &self.layout
    }

    /// The prop currently grabbable by this carrier, if any. Shared by the
    /// grab path and the HUD indicator. Reach is measured to the prop's
    /// surface and works in any direction.
    pub(crate) fn candidate(&self, dynamics: &PropDynamics, carrier: Carrier) -> Option<u32> {
        if self.held.is_some() || carrier.state != PhysicalStateId::BipedGround {
            return None;
        }
        let id = dynamics.nearest_body(carrier.position, self.grab_range())?.0;
        let body = dynamics.held_body(id)?;
        let (frame, record) = self.frame_for(dynamics, id, &body, carrier, &mut None, &mut None)?;
        self.holds(&record, &frame, carrier).then_some(id)
    }

    /// The grab frame and record of prop `id` this tick. Props with authored
    /// grab splines (with `SKATE_PROP_GRAB=1`): the records are rebuilt from
    /// the body's current pose (82D44A10 path A); without a binding (`bound`
    /// `None`) the best record from the skater position (82D4D150 mode 0) is
    /// bound with a new grip (82D444A0 full), with one the record of the same
    /// descriptor keeps its grip (82D444A0 continue); the frame is the record
    /// at the grip (82D45D30). Other props: the interim box edge `edge` (the
    /// face toward the skater when `None`). Bindings are updated in place.
    fn frame_for(
        &self,
        dynamics: &PropDynamics,
        id: u32,
        body: &HeldBody,
        carrier: Carrier,
        bound: &mut Option<skate_core::player::offboard::move_object::HeldGrip>,
        edge: &mut Option<GrabEdge>,
    ) -> Option<(GrabFrame, Record)> {
        use skate_core::player::offboard::{grab_scene::best_spline, move_object};
        let v3 = |v: Vector3| [v.x, v.y, v.z];
        if let Some(records) = authored_records(dynamics, id, self.authored_enabled()) {
            let reference = v3(carrier.skeleton.map_or(carrier.position, |s| s.reference));
            let (record, held) = match *bound {
                None => {
                    let p = carrier.position;
                    let mut record = best_spline(&records, [p.x, p.y, p.z, 1.0])?;
                    let held = move_object::begin_grip(&mut record, reference, self.locomotion.move_object.grab_end_exclusion);
                    (record, held)
                }
                Some(mut held) => {
                    let mut record = records.into_iter().find(|r| (r.0[47], r.0[48]) == held.descriptor)?;
                    move_object::continue_grip(&mut record, reference, &mut held);
                    (record, held)
                }
            };
            *bound = Some(held);
            return Some((authored_frame(&record, &held)?, record));
        }
        let edge = *edge.get_or_insert_with(|| choose_edge(body, carrier.position, self.locomotion.hand_half_spread));
        let frame = grab_frame(body, edge)?;
        let out = scale(frame.forward, -1.0);
        let record = move_object::edge_record(id, v3(frame.ends[0]), v3(frame.ends[1]), v3(out))?;
        Some((frame, record))
    }

    /// Retail hold rule (82D44A10): the held record must pass CanGrabSpline
    /// 82E08EE8 at the grip with the grabbing box and angles. Without skater
    /// observations (tests) it holds.
    fn holds(&self, record: &Record, frame: &GrabFrame, carrier: Carrier) -> bool {
        let Some(skeleton) = carrier.skeleton else { return true };
        let v3 = |v: Vector3| [v.x, v.y, v.z];
        skate_core::player::offboard::move_object::still_holds(
            &self.locomotion.move_object,
            record,
            v3(skeleton.reference),
            frame.grip_distance,
            skeleton.frame,
        )
    }

    fn authored_enabled(&self) -> bool {
        self.authored.unwrap_or_else(super::prop_dynamics::prop_grab_enabled)
    }

    /// Path B of 82D44A10: the prop's best other record from the skater
    /// reference (82D4D150 mode 1 without the held descriptor) that passes
    /// 82E08DB8, bound with a new grip (82D444A0 full). NOT RETAIL YET: the
    /// candidates are the held prop's own records (retail asks the owner's
    /// validated records), owner flag 0x40 and the held record's +196 / +216
    /// are taken as set.
    fn regrab_candidate(&self, dynamics: &PropDynamics, id: u32, skeleton: CarrierSkeleton, held: skate_core::player::offboard::move_object::HeldGrip) -> Option<(GrabFrame, Record, skate_core::player::offboard::move_object::HeldGrip)> {
        use skate_core::player::offboard::{grab_scene::best_spline_excluding, move_object};
        let records = authored_records(dynamics, id, self.authored_enabled())?;
        let r = skeleton.reference;
        let mut record = best_spline_excluding(&records, [r.x, r.y, r.z, 1.0], Some(held.descriptor))?;
        let tuning = &self.locomotion.move_object;
        if !move_object::can_regrab(tuning, &record, [r.x, r.y, r.z], skeleton.frame) {
            return None;
        }
        let new = move_object::begin_grip(&mut record, [r.x, r.y, r.z], tuning.grab_end_exclusion);
        Some((authored_frame(&record, &new)?, record, new))
    }

    /// `PROP_GRAB_MISS` (laptop 2026-10-18: the blue dumpsters never grabbed and left no line): once per grab press
    /// that starts no carry, with the Ground decision (82D324B0: the qualified record, or none) and the nearest prop.
    fn log_miss(&self, dynamics: &PropDynamics, carrier: Carrier, reason: &str) {
        let decision = match carrier.ground_grab {
            Some(Some((kind, key))) => format!("qualified {kind}/{key:#x} (prop {:?} record {})", dynamics.prop_of_grab_key(key, true), key & 63),
            Some(None) => "no record qualified".to_owned(),
            None => "no ground decision".to_owned(),
        };
        let nearest = dynamics.nearest_body(carrier.position, self.grab_range()).map_or_else(
            || "none".to_owned(),
            |(id, centre)| {
                let d = sub(centre, carrier.position);
                format!("{id} ({}) {:.2} m to its centre", dynamics.describe_template(id), dot(d, d).sqrt())
            },
        );
        tracing::info!("PROP_GRAB_MISS {reason}: decision {decision}; nearest prop {nearest}; state {:?}", carrier.state);
    }

    /// The Move Object enter's bind (82D442D0, research b233): with the published record (Player+0x760) passing the
    /// gate (flags_2480 bit 0x400000, geometry +196 and rates identity +216 set), it is copied (82762AB0) and bound
    /// with a new grip (82D444A0 full) at the next carry update. The skater enters 502 either way.
    pub(crate) fn bind_published(&mut self, record: Record) {
        self.pending_bind = Some(record);
    }

    /// Binds a published record: the prop from its spline key, a new grip from bone 23 (82D444A0 full; no hold test,
    /// as in the enter). None (logged) for a record that is not a prop's or a prop that is gone.
    fn bind(&mut self, dynamics: &PropDynamics, mut record: Record, carrier: Carrier) -> Option<u32> {
        let key = record.0[48];
        let id = dynamics.prop_of_grab_key(key, true);
        let Some(id) = id.filter(|&id| dynamics.held_body(id).is_some()) else {
            self.log_miss(dynamics, carrier, "the published record is not a prop's that is still there");
            return None;
        };
        let r = carrier.skeleton.map_or(carrier.position, |s| s.reference);
        let held = skate_core::player::offboard::move_object::begin_grip(&mut record, [r.x, r.y, r.z], self.locomotion.move_object.grab_end_exclusion);
        self.let_go();
        self.held = Some(id);
        self.bound = Some(held);
        self.closest = f32::INFINITY;
        tracing::info!("PROP_GRAB_BIND prop={id} ({}) record {} kind {}", dynamics.describe_template(id), key & 63, record.0[47]);
        Some(id)
    }

    fn let_go(&mut self) {
        self.held = None;
        self.mode = Mode::Carry;
        self.edge = None;
        self.bound = None;
        self.rebind = Default::default();
        self.hands = None;
        self.hand_ik = Default::default();
        self.frame = None;
        self.follow = None;
        self.stick_logged = None;
        self.held_steps = None;
        self.controller = MoveObjectController::default();
    }

    pub(crate) fn update(&mut self, dynamics: &mut PropDynamics, tick: Tick, carrier: Carrier) {
        let on_foot = matches!(
            carrier.state,
            PhysicalStateId::BipedGround | PhysicalStateId::OffBoardPushing
        );
        let pressed = tick.grab && !self.grab_down;
        self.grab_down = tick.grab;
        let Some(id) = self.held else {
            if let Some(record) = self.pending_bind.take() {
                if let Some(id) = self.bind(dynamics, record, carrier) {
                    self.hold(dynamics, id, tick, carrier);
                }
                return;
            }
            // With a Ground decision the carry starts only from the Move Object enter's bind (the decision on tick N,
            // the published record and 502 on N+1, b233); the interim pick is for host tests and the HUD.
            if tick.grab && carrier.ground_grab.is_none() {
                if let Some(id) = self.candidate(dynamics, carrier) {
                    self.let_go();
                    self.held = Some(id);
                    self.closest = f32::INFINITY;
                    self.hold(dynamics, id, tick, carrier);
                }
            } else if pressed && carrier.state == PhysicalStateId::BipedGround && carrier.ground_grab == Some(None) {
                self.log_miss(dynamics, carrier, "no validated record passed 82E08DB8");
            }
            return;
        };
        if !on_foot || dynamics.position_of(id).is_none() {
            // Auto-drop: never a confirmed placement, so nothing is saved.
            self.let_go();
            return;
        }
        match self.mode {
            Mode::Carry => {
                if !tick.grab {
                    // Velocity is kept: releasing while moving throws gently.
                    self.let_go();
                    return;
                }
                if tick.placement {
                    // Enter placement at the prop's current relative pose so
                    // the ghost starts where the drag left it.
                    let position = dynamics.position_of(id).unwrap_or(carrier.position);
                    let offset = sub(position, carrier.position);
                    let flat = Vector3::new(offset.x, 0.0, offset.z);
                    self.mode = Mode::Placement {
                        distance: dot(flat, flat)
                            .sqrt()
                            .clamp(PLACE_DISTANCE.start, PLACE_DISTANCE.end),
                        height: offset.y.clamp(PLACE_HEIGHT.start, PLACE_HEIGHT.end),
                        yaw: 0.0,
                    };
                    self.frame = None;
                    return;
                }
                self.hold(dynamics, id, tick, carrier);
            }
            Mode::Placement {
                mut distance,
                mut height,
                mut yaw,
            } => {
                if !tick.grab {
                    self.confirm(dynamics, id);
                    self.let_go();
                    return;
                }
                if tick.placement {
                    self.mode = Mode::Carry;
                    self.edge = None;
                    self.bound = None;
                    self.follow = None;
                    self.controller = MoveObjectController::default();
                    self.closest = f32::INFINITY;
                    return;
                }
                let dt = carrier.time_step;
                yaw += tick.yaw_axis * PLACE_YAW_RATE * dt;
                distance = (distance + tick.distance_axis * PLACE_DISTANCE_RATE * dt)
                    .clamp(PLACE_DISTANCE.start, PLACE_DISTANCE.end);
                height = (height + tick.height_axis * PLACE_HEIGHT_RATE * dt)
                    .clamp(PLACE_HEIGHT.start, PLACE_HEIGHT.end);
                self.mode = Mode::Placement {
                    distance,
                    height,
                    yaw,
                };
                let (target, basis) = ghost_pose(carrier, distance, height, yaw);
                dynamics.carry_to_pose(id, target, basis, MAX_CARRY_SPEED, dt);
            }
        }
    }

    /// Move Object hold (retail state 502 update 82D44A10): the grab record is
    /// refreshed from the prop's current pose, the retail command (82D45318)
    /// goes to the prop through `apply_move_command`, and the grab frame the
    /// skater follows (82D45D30) is published for `biped_ground`. The prop
    /// leads; the skater follows it.
    fn hold(&mut self, dynamics: &mut PropDynamics, id: u32, tick: Tick, carrier: Carrier) {
        let Some(body) = dynamics.held_body(id) else {
            self.let_go();
            return;
        };
        let locomotion = self.locomotion;
        let tuning = locomotion.command_tuning();
        let was_bound = self.bound;
        let (mut bound, mut edge) = (self.bound, self.edge);
        let found = self.frame_for(dynamics, id, &body, carrier, &mut bound, &mut edge);
        let holds = found.as_ref().is_some_and(|(f, r)| self.holds(r, f, carrier));
        let mut regrabbed = false;
        let (mut frame, record) = match (was_bound, carrier.skeleton) {
            // An authored record held since last tick: the rebind block (82D44A10, b64).
            (Some(held), Some(skeleton)) => {
                use skate_core::player::offboard::move_object::{Rebind, RebindInput};
                let rt = self.rebind_tuning;
                // The push latch input (|Player+736|^2) is not identified: never set here.
                self.rebind.tick(&rt, 0.0, skeleton.collision_flag, tuning.tick);
                let candidate = if found.is_some() && holds { None } else { self.regrab_candidate(dynamics, id, skeleton, held) };
                let input = RebindInput { hand_flag: tick.grab, has_best: found.is_some(), still_holds: holds, owner_ready: true, candidate: candidate.is_some(), held_fields: true, was_holding: true };
                match self.rebind.decide(&rt, &input) {
                    Rebind::Refresh => found.expect("refresh needs the held record"),
                    Rebind::Regrab { .. } => {
                        let (f, r, new) = candidate.expect("regrab needs a candidate");
                        bound = Some(new);
                        regrabbed = true;
                        (f, r)
                    }
                    Rebind::Lost => {
                        self.let_go();
                        return;
                    }
                }
            }
            // Box stand-in, the grab tick, or no skater observations: the
            // held record must keep qualifying (82D44A10 -> 82E08EE8).
            _ => match found {
                Some(x) if holds => x,
                _ => {
                    self.let_go();
                    return;
                }
            },
        };
        (self.bound, self.edge) = (bound, edge);
        // 82D45D30 hand points on the authored record; 82D46610 / 82D45008 the
        // hand IK (a path B re-grab is 82D444A0 full: the bit clears).
        self.hands = match (self.bound, carrier.skeleton) {
            (Some(held), Some(s)) => skate_core::player::offboard::move_object::hand_points(&record, held.grip, [0.0; 3], [s.hand_span, 0.0, 0.0], &self.rebind_tuning),
            _ => None,
        };
        if regrabbed {
            self.hand_ik.begin_grab();
        }
        if let Some(s) = carrier.skeleton {
            self.hand_ik.tick(&self.locomotion.move_object, s.state_time);
        }
        let v3 = |v: Vector3| [v.x, v.y, v.z];
        let body_position = carrier.skeleton.map_or(carrier.position, |s| s.body);
        // +368 is the latched frame row pointing from the edge toward the
        // skater (minus our "into the object" forward).
        let into = v3(frame.forward);
        let back_of = |c: &MoveObjectController| {
            let f = if c.latched { c.latched_forward } else { into };
            [-f[0], 0.0, -f[2]]
        };
        // 82D442D0 / 82D444A0: follow point = body, anchor from the new grip.
        let follow = self
            .follow
            .get_or_insert_with(|| SkaterFollow::begin(&tuning, v3(body_position), v3(frame.grip), back_of(&MoveObjectController::default())));
        // Path B (82D444A0 full after a re-grab): anchor velocity +624 = 0 and
        // the anchor seeded from the new record's nearest point (82D43B20).
        // The frame blend 82D46218 only runs for jumps of 60 m or more: a
        // snap here, as for the grab.
        if regrabbed {
            if let (Some(s), Some(_)) = (carrier.skeleton, self.bound) {
                use skate_core::player::offboard::grab_scene::{at_distance, nearest_distance};
                let r = s.reference;
                let s_near = nearest_distance(&record, [r.x, r.y, r.z, 1.0]);
                let near = at_distance(&record, s_near);
                let back = back_of(&self.controller);
                follow.anchor = skate_core::player::offboard::move_object::seed_anchor([near[0], near[1], near[2]], back, tuning.anchor_reach);
                follow.velocity = [0.0; 3];
            }
        }
        // 82D46610 runs before the command with the previous tick's latch.
        follow.update_anchor(&tuning, v3(frame.grip), back_of(&self.controller));
        let input = MoveObjectInput {
            move_z: tick.object_move[1],
            move_x: tick.object_move[0],
            move_rotation: tick.object_move[2],
            forward: v3(frame.forward),
            grip: v3(frame.grip),
            center: v3(body.center),
            velocity: v3(body.velocity),
            heading: body.heading(),
            mass: body.mass,
            yaw_inertia: body.yaw_inertia,
            contact_normal: v3(tick.contact_normal),
            record_272: body.record_272,
        };
        let command = skate_core::player::offboard::move_object::command(&tuning, &mut self.controller, &input);
        // CARRY_STICK (laptop 2026-10-18: the log had no stick input while carrying): on change, the stick
        // (push / pull, side, turn), the skater's facing and right, and where the target velocity points.
        let stick = [input.move_z, input.move_x, input.move_rotation].map(|v| (v * 10.0).round() as i32);
        if self.stick_logged != Some(stick) {
            self.stick_logged = Some(stick);
            let (f, v) = (input.forward, command.target_velocity);
            tracing::info!(
                "CARRY_STICK prop={id} push={:.1} side={:.1} turn={:.1} facing=[{:.2}, {:.2}] right=[{:.2}, {:.2}] target_velocity=[{:.2}, {:.2}] yaw_target={:.2}",
                input.move_z, input.move_x, input.move_rotation, f[0], f[2], -f[2], f[0], v[0], v[2], command.yaw_target
            );
        }
        // CARRY_WEIGHT once per grab (b284: a held prop's weight is only these two curve outputs, set at the grab;
        // a light dumpster means a light mass or yaw inertia here). CARRY_ORBIT while turning: retail turns the prop
        // about its centre and the skater walks round it, so the skater's angle round the centre should follow the
        // prop's heading.
        let steps = self.held_steps.map_or(0, |s| s + 1);
        if self.held_steps.is_none() {
            tracing::info!(
                "CARRY_WEIGHT prop={id} mass={:.1} yaw_inertia={:.1} speed_scale={:.2} yaw_gain={:.2}",
                input.mass, input.yaw_inertia, tuning.speed_scale(input.mass), tuning.yaw_gain(input.yaw_inertia)
            );
        }
        self.held_steps = Some(steps);
        if input.move_rotation.abs() > 0.05 && steps % 15 == 0 {
            let d = [body_position.x - input.center[0], body_position.z - input.center[2]];
            tracing::info!(
                "CARRY_ORBIT prop={id} turn={:.1} prop_heading={:.3} skater_angle={:.3} radius={:.2} yaw_target={:.2} step={steps}",
                input.move_rotation, input.heading, d[0].atan2(d[1]), d[0].hypot(d[1]), command.yaw_target
            );
        }
        // Follow step after the command (this tick's latch): +416 steps
        // toward the edge and that step is the displacement handed to the
        // character move (82D44A10 -> 82BDF268). NOT RETAIL YET: 82BDF268
        // itself (sweep, step-up, weight +1124); here the skater root is
        // moved by the same step (the walking job's velocity override in
        // biped_ground), so the live body offset never feeds back.
        let back = back_of(&self.controller);
        let root_y = carrier.skeleton.map_or(carrier.position.y, |s| s.frame[3][1]);
        let step = self
            .follow
            .as_mut()
            .map(|f| {
                let before = f.point;
                let after = f.step(&tuning, v3(frame.grip), back, root_y);
                [after[0] - before[0], after[2] - before[2]]
            })
            .unwrap_or([0.0, 0.0]);
        frame.skater = Vector3::new(carrier.position.x + step[0], carrier.position.y, carrier.position.z + step[1]);
        // Optional engine let-go for a mod (`let_go_distance` > 0).
        if locomotion.let_go_distance > 0.0 {
            let behind = flat_len(sub(frame.skater, carrier.position));
            self.closest = self.closest.min(behind);
            if behind > self.closest + locomotion.let_go_distance {
                self.let_go();
                return;
            }
        }
        let linear = Vector3::new(command.linear[0], command.linear[1], command.linear[2]);
        dynamics.apply_move_command(id, linear, command.yaw, frame.grip, carrier.time_step);
        dynamics.set_move_diagnostics(Some(super::prop_dynamics::MoveDiagnostics {
            stick: tick.object_move,
            linear: command.linear,
            yaw: command.yaw,
            lever: command.lever,
            rotation: command.rotation_demand,
            blocked: command.blocked,
            drift: command.drift,
            yaw_rate: command.yaw_rate,
        }));
        self.last = command;
        self.last_input = tick.object_move;
        self.frame = Some(frame);
    }

    /// Record the confirmed pose and rewrite the layout sidecar.
    fn confirm(&mut self, dynamics: &mut PropDynamics, id: u32) {
        dynamics.release_still(id);
        let Some((origin, basis)) = dynamics.pose(id) else {
            return;
        };
        self.layout.insert(
            id,
            super::prop_layout::PropPose {
                id,
                origin: [origin.x, origin.y, origin.z],
                basis: basis.columns,
            },
        );
        if let Some(path) = &self.layout_path {
            if let Err(error) = super::prop_layout::save(path, &self.layout) {
                warn!("SKATE_PROP_LAYOUT: {}: {error}", path.display());
            }
        }
    }
}

/// Ghost pose: `distance` ahead of the carrier along facing+yaw, `height`
/// above the root, rotated `yaw` from the carrier's facing.
fn ghost_pose(
    carrier: Carrier,
    distance: f32,
    height: f32,
    yaw: f32,
) -> (Vector3, skate_core::math::Basis3) {
    let angle = carrier.facing_yaw() + yaw;
    let (sin, cos) = angle.sin_cos();
    let target = add(
        carrier.position,
        Vector3::new(sin * distance, height, cos * distance),
    );
    let basis = skate_core::math::Basis3 {
        columns: [[cos, 0.0, -sin], [0.0, 1.0, 0.0], [sin, 0.0, cos]],
    };
    (target, basis)
}

fn add(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}

fn sub(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn scale(v: Vector3, s: f32) -> Vector3 {
    Vector3::new(v.x * s, v.y * s, v.z * s)
}

fn dot(a: Vector3, b: Vector3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn flat_len(v: Vector3) -> f32 {
    (v.x * v.x + v.z * v.z).sqrt()
}

fn flat_unit(v: Vector3) -> Option<Vector3> {
    let l = flat_len(v);
    (l > 1e-6).then(|| Vector3::new(v.x / l, 0.0, v.z / l))
}

/// Rotate a ground-frame row about world +Y by `angle` (positive turns +Z
/// toward +X).
pub(crate) fn yaw_row(v: [f32; 4], angle: f32) -> [f32; 4] {
    let (sin, cos) = angle.sin_cos();
    [v[0] * cos + v[2] * sin, v[1], -v[0] * sin + v[2] * cos, v[3]]
}
