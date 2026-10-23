//! Instant replay of every drawn actor (upstream PR #52 `b110bab` `replay/world.rs`, Bevy-free; step 1 of its port:
//! recording and its cost only, playback still from `session_replay`). The sim thread records, after each tick, the
//! skater, NPC skaters, peds, traffic cars, streamed-in props and released hand props as drawn into
//! `skate_core::replay_buffer` (retail's DataTracker: presentation data, never a re-simulation; local joint
//! rotations as i16 quaternions, a key only when the quantised pose changed).
//!
//! Playback ([`fill`]): during a replay the app draws the world from the recording at the replay's time, every
//! recorded actor at full opacity and LOD 0 (as upstream: no draw distance fade in a replay). What the poses do not
//! carry (a ped's model, tints and held prop, a car's model and paint, a prop's mesh range) is kept per actor as a
//! history of changes ([`Look`]), so a ped holds what it held at the replay's time (upstream records a held prop as an
//! actor of its own; ours rides with the ped's look in the hand bone's frame, the same picture).
//!
//! Ours: the byte budget is the Vita's ([`BUDGET`]; upstream's default is 128 MB), so a busy world keeps a shorter
//! window than retail's 90 s. Bones arrive as model-space matrices: each joint's local transform is its parent's
//! inverse times its own. A car's wheel angle rides as the rotation of its one joint. A released hand prop is keyed
//! by its prop body id.
#![cfg_attr(not(target_os = "vita"), allow(dead_code))]
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use glam::{Affine3A, Mat4, Quat, Vec3};
use skate_core::replay_buffer::{ActorPose, JointPose, ReplayBuffer, ReplayLimits, RETAIL_MAX_FRAMES};
use skate_sim::live::Session;

/// The recording's byte budget on the Vita (measured in DownTown before it is raised or lowered).
pub const BUDGET: usize = 24 << 20;

/// Actor id tags (high bits); the low bits are the actor's own serial or id.
const SKATER: u64 = 0;
const NPC: u64 = 1 << 60;
const PED: u64 = 2 << 60;
const CAR: u64 = 3 << 60;
const PROP: u64 = 4 << 60;
const HAND_PROP: u64 = 5 << 60;

/// Ticks between two `WORLD_REPLAY` log lines (10 s).
const LOG_TICKS: u64 = 600;

/// What an actor is drawn with besides its pose.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Npc,
    Ped { model: u16, tint_a: [f32; 4], tint_b: [f32; 4], hand: Option<(u16, Mat4)> },
    Car { model: u16, chassis: [f32; 4], secondary: [f32; 4], paint_gain: f32 },
    Prop { first_index: u32, index_count: u32 },
    HandProp { model: u16 },
}

pub struct WorldRecorder {
    pub buffer: ReplayBuffer,
    /// Each actor's looks from the time they took effect (oldest first; one entry while it does not change).
    looks: std::collections::HashMap<u64, VecDeque<(f64, Look)>>,
    /// The rigs' parents (stock skater rig, ped rig) for playback.
    skater_parents: Vec<i32>,
    ped_parents: Vec<i32>,
    /// Actors recorded last tick (sorted), to end the tracks of those gone.
    seen: Vec<u64>,
    now: Vec<u64>,
    /// Scratch: the frame's inverse globals (bones are affine) and one actor's pose.
    inverse: Vec<Affine3A>,
    pose: ActorPose,
    /// Recording time over the log period: total, worst tick, ticks.
    spent_us: u64,
    worst_us: u32,
    /// The worst tick's share in `ReplayBuffer::begin_frame` (eviction), and ticks over 5 ms.
    worst_begin_us: u32,
    slow: u32,
    ticks: u64,
}

pub static RECORDER: Mutex<Option<WorldRecorder>> = Mutex::new(None);

impl WorldRecorder {
    fn new() -> Self {
        Self {
            buffer: ReplayBuffer::new(ReplayLimits { max_frames: RETAIL_MAX_FRAMES, byte_budget: BUDGET }),
            looks: std::collections::HashMap::new(),
            skater_parents: Vec::new(),
            ped_parents: Vec::new(),
            seen: Vec::new(),
            now: Vec::new(),
            inverse: Vec::new(),
            pose: ActorPose::default(),
            spent_us: 0,
            worst_us: 0,
            worst_begin_us: 0,
            slow: 0,
            ticks: 0,
        }
    }

    /// `look` for `id` from the current frame on (stored only when it changed).
    fn look(&mut self, id: u64, look: Look) {
        let Some((_, now)) = self.buffer.bounds() else { return };
        let history = self.looks.entry(id).or_default();
        if history.back().is_none_or(|(_, last)| *last != look) {
            history.push_back((now, look));
        }
    }

    fn put(&mut self, id: u64) {
        self.now.push(id);
        self.buffer.record(id, &self.pose);
    }

    /// The pose of an actor drawn at `root` with model-space `globals` over a rig of `parents`.
    fn skeleton(&mut self, id: u64, root: JointPose, globals: &[Mat4], parents: &[i32]) {
        self.inverse.clear();
        self.inverse.extend(globals.iter().map(|g| Affine3A::from_mat4(*g).inverse()));
        self.pose.root = root;
        self.pose.hidden = false;
        self.pose.joints.clear();
        for (i, g) in globals.iter().enumerate() {
            let parent = parents.get(i).copied().unwrap_or(-1);
            let local = match usize::try_from(parent).ok().and_then(|p| self.inverse.get(p)) {
                Some(inverse) => Mat4::from(*inverse) * *g,
                None => *g,
            };
            self.pose.joints.push(joint(&local));
        }
        self.put(id);
    }

    /// An actor with a root only (`joints` extra joint rotations, e.g. a car's wheel).
    fn rigid(&mut self, id: u64, root: JointPose, joints: &[Quat]) {
        self.pose.root = root;
        self.pose.hidden = false;
        self.pose.joints.clear();
        self.pose.joints.extend(joints.iter().map(|q| JointPose { rotation: q.to_array(), ..JointPose::default() }));
        self.put(id);
    }
}

fn joint(m: &Mat4) -> JointPose {
    let (scale, rotation, translation) = m.to_scale_rotation_translation();
    JointPose { translation: translation.to_array(), rotation: rotation.to_array(), scale: scale.to_array() }
}

fn rooted(translation: Vec3, rotation: Quat) -> JointPose {
    JointPose { translation: translation.to_array(), rotation: rotation.to_array(), scale: [1.0; 3] }
}

/// A new session or world: the old recording belongs to another one.
pub fn clear() {
    if let Some(r) = RECORDER.lock().unwrap().as_mut() {
        r.buffer.clear();
        r.seen.clear();
        r.looks.clear();
    }
}

/// Called by the sim thread after each recorded tick, with the poses it publishes for drawing.
#[allow(clippy::too_many_arguments)]
pub fn record(
    session: &Session,
    cars: &[skate_sim::CarPose],
    npcs: &[skate_sim::NpcPose],
    peds: &[skate_sim::PedPose],
    ped_parents: &[i32],
    hand_props: &[skate_sim::HandPropPose],
    props: &[skate_sim::live::PropPose],
) {
    let Some((_, skater)) = session.presentation() else { return };
    let start = Instant::now();
    let mut guard = RECORDER.lock().unwrap();
    let r = guard.get_or_insert_with(WorldRecorder::new);
    r.buffer.begin_frame(session.ticks() as f64 * session.period().as_secs_f64());
    let begin_us = start.elapsed().as_micros() as u32;
    r.now.clear();
    let parents = session.bone_parents();
    if r.skater_parents.len() != parents.len() || r.ped_parents.len() != ped_parents.len() {
        r.skater_parents = parents.to_vec();
        r.ped_parents = ped_parents.to_vec();
    }
    r.skeleton(SKATER, rooted(skater.root.translation, skater.root.rotation), &skater.bones, parents);
    for n in npcs {
        let c = &n.current;
        r.look(NPC | u64::from(n.serial), Look::Npc);
        r.skeleton(NPC | u64::from(n.serial), rooted(c.root.translation, c.root.rotation), &c.bones, parents);
    }
    for p in peds {
        let (at, heading, globals) = &p.current;
        r.look(PED | u64::from(p.serial), Look::Ped { model: p.model, tint_a: p.tint_a, tint_b: p.tint_b, hand: p.hand });
        r.skeleton(PED | u64::from(p.serial), rooted(*at, Quat::from_rotation_y(*heading)), globals, ped_parents);
    }
    for c in cars {
        let (at, rotation) = c.curr;
        r.look(CAR | u64::from(c.serial), Look::Car { model: c.model, chassis: c.chassis, secondary: c.secondary, paint_gain: c.paint_gain });
        r.rigid(CAR | u64::from(c.serial), rooted(Vec3::from_array(at), Quat::from_array(rotation)), &[Quat::from_rotation_x(c.wheel)]);
    }
    for p in props {
        let basis = glam::Mat3::from_cols_array_2d(&p.basis);
        let root = JointPose { translation: p.origin, rotation: Quat::from_mat3(&basis).to_array(), scale: p.scale };
        r.look(PROP | u64::from(p.id), Look::Prop { first_index: p.first_index, index_count: p.index_count });
        r.rigid(PROP | u64::from(p.id), root, &[]);
    }
    for h in hand_props {
        r.look(HAND_PROP | u64::from(h.id), Look::HandProp { model: h.model });
        r.rigid(HAND_PROP | u64::from(h.id), joint(&h.current), &[]);
    }
    // Actors gone since the last tick end their tracks here.
    r.now.sort_unstable();
    let WorldRecorder { buffer, seen, now, .. } = &mut *r;
    for id in seen.iter().filter(|id| now.binary_search(id).is_err()) {
        buffer.end(*id);
    }
    std::mem::swap(seen, now);
    let us = start.elapsed().as_micros() as u32;
    r.spent_us += u64::from(us);
    if us > r.worst_us {
        (r.worst_us, r.worst_begin_us) = (us, begin_us);
    }
    r.slow += u32::from(us > 5000);
    r.ticks += 1;
    if r.ticks % LOG_TICKS == 0 {
        let seconds = r.buffer.bounds().map_or(0.0, |(a, b)| b - a);
        println!(
            "WORLD_REPLAY frames={} seconds={seconds:.1} bytes={} ({:.1} MB of {}) actors={} rec_ms avg={:.3} max={:.3} (begin {:.3}) over_5ms={} looks={}",
            r.buffer.frame_count(),
            r.buffer.bytes(),
            r.buffer.bytes() as f64 / (1 << 20) as f64,
            BUDGET >> 20,
            r.seen.len(),
            r.spent_us as f64 / LOG_TICKS as f64 / 1000.0,
            r.worst_us as f64 / 1000.0,
            r.worst_begin_us as f64 / 1000.0,
            r.slow,
            r.looks.values().map(VecDeque::len).sum::<usize>(),
        );
        (r.spent_us, r.worst_us, r.worst_begin_us, r.slow) = (0, 0, 0, 0);
        // The looks of actors whose tracks the window dropped, and looks replaced before the window's start.
        let WorldRecorder { buffer, looks, .. } = &mut *r;
        let start = buffer.bounds().map_or(0.0, |(a, _)| a);
        looks.retain(|id, history| {
            while history.len() >= 2 && history[1].0 <= start {
                history.pop_front();
            }
            buffer.contains(*id)
        });
    }
}

/// The world's draw poses at `time` (the replay clock), into the app's lists (cleared first; previous and current
/// are the same pose: the recording already blends between its keys). False when nothing is recorded there.
pub fn fill(
    time: f64,
    cars: &mut Vec<skate_sim::CarPose>,
    npcs: &mut Vec<skate_sim::NpcPose>,
    peds: &mut Vec<skate_sim::PedPose>,
    hand_props: &mut Vec<skate_sim::HandPropPose>,
    props: &mut Vec<skate_sim::live::PropPose>,
) -> bool {
    let guard = RECORDER.lock().unwrap();
    let Some(r) = guard.as_ref() else { return false };
    if r.buffer.bounds().is_none_or(|(a, b)| time < a - 1e-6 || time > b + 1e-6) {
        return false;
    }
    cars.clear();
    npcs.clear();
    peds.clear();
    hand_props.clear();
    props.clear();
    for id in r.buffer.ids() {
        let Some(history) = r.looks.get(&id) else { continue };
        // The look in effect at `time` (the oldest one before the first change).
        let at = history.partition_point(|(t, _)| *t <= time).max(1) - 1;
        let (Some((_, look)), Some(pose)) = (history.get(at), r.buffer.sample(id, time)) else { continue };
        let serial = (id & 0xFFFF_FFFF) as u32;
        let root_t = Vec3::from_array(pose.root.translation);
        let root_q = Quat::from_array(pose.root.rotation);
        match *look {
            Look::Npc => {
                let snapshot = skate_sim::live::Snapshot {
                    root: skate_sim::live::Transform { translation: root_t, rotation: root_q, scale: Vec3::ONE },
                    bones: globals(&pose.joints, &r.skater_parents),
                    camera: skate_sim::live::Transform::IDENTITY,
                    fov: 1.0,
                };
                npcs.push(skate_sim::NpcPose { serial, previous: snapshot.clone(), current: snapshot, alpha: 1.0 });
            }
            Look::Ped { model, tint_a, tint_b, hand } => {
                let (axis, angle) = root_q.to_axis_angle();
                let heading = if axis.y < 0.0 { -angle } else { angle };
                let drawn = (root_t, heading, globals(&pose.joints, &r.ped_parents));
                peds.push(skate_sim::PedPose { serial, model, previous: drawn.clone(), current: drawn, tint_a, tint_b, alpha: 1.0, lod: 0, hand });
            }
            Look::Car { model, chassis, secondary, paint_gain } => {
                let wheel = pose.joints.first().map_or(0.0, |j| {
                    let q = Quat::from_array(j.rotation);
                    (2.0 * q.x.atan2(q.w)).rem_euclid(std::f32::consts::TAU)
                });
                let at = (pose.root.translation, pose.root.rotation);
                cars.push(skate_sim::CarPose { serial, model, prev: at, curr: at, wheel_prev: wheel, wheel, chassis, secondary, paint_gain });
            }
            Look::Prop { first_index, index_count } => {
                let basis = glam::Mat3::from_quat(root_q);
                props.push(skate_sim::live::PropPose {
                    id: serial,
                    first_index,
                    index_count,
                    origin: pose.root.translation,
                    basis: basis.to_cols_array_2d(),
                    scale: pose.root.scale,
                });
            }
            Look::HandProp { model } => {
                let m = Mat4::from_scale_rotation_translation(Vec3::from_array(pose.root.scale), root_q, root_t);
                hand_props.push(skate_sim::HandPropPose { id: serial, model, previous: m, current: m });
            }
        }
    }
    true
}

/// Model-space bone matrices from local joints over a rig of `parents` (a parent before its children).
fn globals(joints: &[JointPose], parents: &[i32]) -> Vec<Mat4> {
    let mut out: Vec<Mat4> = Vec::with_capacity(joints.len());
    for (i, j) in joints.iter().enumerate() {
        let local = Mat4::from_scale_rotation_translation(Vec3::from_array(j.scale), Quat::from_array(j.rotation), Vec3::from_array(j.translation));
        let parent = parents.get(i).and_then(|&p| usize::try_from(p).ok()).filter(|&p| p < i);
        out.push(match parent {
            Some(p) => out[p] * local,
            None => local,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locals from model-space globals compose back to the globals (what playback will rebuild).
    #[test]
    fn locals_compose_back_to_the_globals() {
        let parents = [-1, 0, 1];
        let locals = [
            Mat4::from_rotation_translation(Quat::from_rotation_y(0.3), Vec3::new(0.0, 1.0, 0.0)),
            Mat4::from_rotation_translation(Quat::from_rotation_x(-0.7), Vec3::new(0.0, 0.4, 0.1)),
            Mat4::from_rotation_translation(Quat::from_rotation_z(1.1), Vec3::new(0.2, 0.3, 0.0)),
        ];
        let globals = [locals[0], locals[0] * locals[1], locals[0] * locals[1] * locals[2]];
        let mut r = WorldRecorder::new();
        r.buffer.begin_frame(0.0);
        r.skeleton(7, JointPose::default(), &globals, &parents);
        let pose = r.buffer.sample(7, 0.0).unwrap();
        let composed = super::globals(&pose.joints, &parents);
        for (a, b) in composed.iter().zip(&globals) {
            assert!(a.abs_diff_eq(*b, 1e-3), "{a:?} != {b:?}");
        }
    }
}
