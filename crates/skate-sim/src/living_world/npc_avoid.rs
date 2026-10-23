//! NPC skaters avoid peds, cars, props and other skaters with retail's obstacle avoider
//! (`skate_core::living_world::avoid`, doc 26 "NPC skater obstacle avoider").
//!
//! Each world tick, before the cursors advance, the host gathers the obstacles (peds and their
//! velocity from the last tick, traffic cars with their box, props with their box, the other
//! NPC skaters and the players) and runs the avoider for every riding NPC skater. Its answer goes
//! where retail's goes:
//! - the speed (retail: the AI record speed, `sub_82470830`): the replay tier holds the cursor
//!   back by the missing recorded frames (`NpcAvoid::lag`), so a cap of 0 stops the skater and a
//!   floor only catches up held-back frames (a recording is never played faster than recorded);
//! - the steering (retail: the target moved across the path, `sub_82464E30`, and the board pulled
//!   towards it at 2 cm a tick, `82C05EC0`): a lateral offset of the drawn skater that moves 2 cm
//!   a tick towards the chosen gap and back to the line once the path is clear.
//! The simulated tier reads the same output for its AI record (`npc_sim`).
//!
//! Air, off-board and trick spans are never held back or moved (the recorded jump would break).
//! Multiplayer: only the host evaluates; `lag` and `offset` are plain per-NPC state a client
//! would take from the host (no replication yet). Logs `NPC_AVOID` on every mode change.

use super::npc_skaters::{NpcEntity, NpcSkaterEvent};
use super::{LivingWorldSettings, PopulationState};
use glam::Vec3;
use skate_core::living_world::avoid::{self, AvoidMode, AvoidOutput, AvoidSelf, AvoidState, Obstacle, ObstacleKind};
use skate_core::living_world::replay::{node_flags, project_on_line};
use std::collections::BTreeMap;

/// Board position step of the AI board path, metres per 60 Hz tick (`82C05EC0`, 2 cm).
pub(crate) const LATERAL_STEP: f32 = 0.02;

/// Retail's skater obstacle size is not decoded per gatherer; the skater gatherer
/// (`sub_82464968`) reads 0.75 (`0x821814A0`) [inferred: footprint].
pub(crate) const SKATER_SIZE: f32 = 0.75;

/// A sample that moved farther than this in one tick jumped (respawn, teleport, line switch).
const MAX_TICK_MOVE: f32 = 2.0;

/// One NPC skater's avoider state (part of [`NpcReplay`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NpcAvoid {
    pub state: AvoidState,
    pub last: AvoidOutput,
    /// Recorded 60 Hz frames the cursor is held back (fractional; the cursor uses the whole part).
    pub lag: f32,
    /// Lateral offset of the drawn skater, metres right of the line, now and one tick back.
    pub offset: f32,
    pub previous_offset: f32,
    /// The controller sub-mode state (`ctrl+568` ..., `skate_core::living_world::ai_controller`) and the line it
    /// belongs to (a line change or the line's end resets it, `sub_82468AC8`; b70).
    pub controller: skate_core::living_world::ai_controller::ControllerState,
    pub controller_line: Option<[u8; 16]>,
    /// Last tick's sample position: the skater's actual motion for the avoider (b112).
    pub moved_from: Option<[f32; 3]>,
}

impl NpcAvoid {
    /// Controller sub-mode 4 (mode 6, a low prop): no speed shaping, no recorded node actions (b66).
    pub fn low_prop(&self) -> bool {
        self.controller.sub_mode == skate_core::living_world::ai_controller::sub_mode::LOW_PROP
    }
}

impl NpcAvoid {
    /// Whole frames to hold the cursor back.
    pub fn held_frames(&self) -> u64 {
        self.lag.max(0.0).floor() as u64
    }
    /// The drawn offset `fraction` of the way through the tick.
    pub fn offset_at(&self, fraction: f32) -> f32 {
        self.previous_offset + (self.offset - self.previous_offset) * fraction.clamp(0.0, 1.0)
    }
}

/// Positions one tick back (ped velocities; peds carry no velocity).
#[derive(Default)]
pub(crate) struct AvoidTrack {
    tick: u64,
    previous: BTreeMap<u64, [f32; 3]>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Right of a direction in the ground plane (the same axis as `avoid::steer_point`).
pub(crate) fn right_of(d: [f32; 3]) -> [f32; 3] {
    let l = d[0].hypot(d[2]);
    if l > 1e-6 {
        [-d[2] / l, 0.0, d[0] / l]
    } else {
        [1.0, 0.0, 0.0]
    }
}

fn circle(id: u64, kind: ObstacleKind, position: [f32; 3], velocity: [f32; 3], size: f32, height: f32) -> Obstacle {
    let forward = if length(velocity) > 1e-3 { velocity.map(|c| c / length(velocity)) } else { [0.0, 0.0, 1.0] };
    Obstacle { id, kind, position, velocity, axes: [right_of(forward), [0.0, 1.0, 0.0], forward], length: size, width: size, height }
}

/// Player obstacle ids (above every living-world id).
const PLAYER_ID_BASE: u64 = u64::MAX - 64;

/// One tick of the avoider (`avoid`): `cars` = (id, transform, bounds, velocity) of every traffic
/// car, `props` = the props' obstacle boxes, `observers` = the players; `dt` = the tick's time.
#[allow(clippy::too_many_arguments)]
pub(crate) fn avoid(
    settings: &LivingWorldSettings,
    observers: &[skate_core::living_world::Observer],
    state: &PopulationState,
    track: &mut AvoidTrack,
    dt: f32,
    peds: &[(u64, [f32; 3])],
    ped_agent: Option<(f32, f32)>,
    cars: &[(u64, crate::transform::Transform, [[f32; 3]; 2], Vec3)],
    props: &[skate_core::living_world::peds::ObstacleInput],
    npcs: &mut [&mut NpcEntity],
    events: &mut Vec<NpcSkaterEvent>,
) {
    let _scope = skate_core::profile::scope("z_npc_avoid");
    let s = settings.npc_avoid;
    let tick = state.world.tick();
    let mut obstacles: Vec<Obstacle> = Vec::new();
    let elapsed = (tick.saturating_sub(track.tick)) as f32 / (state.world.clock().hz as f32).max(1.0);
    let mut seen = BTreeMap::new();
    if s.enabled {
        // Peds: NavPower agent radius and height (the ped body's own data).
        let (radius, height) = ped_agent.unwrap_or((super::vehicle_contacts::FALLBACK_PED_RADIUS, super::vehicle_contacts::FALLBACK_PED_HEIGHT));
        for &(id, p) in peds {
            let v = match track.previous.get(&id) {
                Some(q) if elapsed > 0.0 => sub(p, *q).map(|c| c / elapsed),
                _ => [0.0; 3],
            };
            seen.insert(id, p);
            obstacles.push(circle(id, ObstacleKind::Pedestrian, p, v, radius * 2.0, height));
        }
        // Cars: the model's bounds around the current pose.
        for (id, t, bounds, velocity) in cars {
            let [lo, hi] = *bounds;
            let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
            let centre_local = Vec3::new((hi[0] + lo[0]) * 0.5, (hi[1] + lo[1]) * 0.5, (hi[2] + lo[2]) * 0.5);
            let axes = [t.rotation * Vec3::X, t.rotation * Vec3::Y, t.rotation * Vec3::Z].map(|a| a.to_array());
            obstacles.push(Obstacle {
                id: *id,
                kind: ObstacleKind::Vehicle,
                position: (t.translation + t.rotation * centre_local).to_array(),
                velocity: velocity.to_array(),
                axes,
                length: size[2],
                width: size[0],
                height: size[1],
            });
        }
        // Props: their collision boxes (held ones too, like the ped obstacles).
        for o in props {
            if o.inactive {
                continue;
            }
            obstacles.push(Obstacle {
                id: o.id,
                kind: ObstacleKind::Prop,
                position: o.center,
                velocity: o.velocity,
                axes: o.axes,
                length: o.half_extents[2] * 2.0,
                width: o.half_extents[0] * 2.0,
                height: o.half_extents[1] * 2.0,
            });
        }
        // Players.
        for (i, o) in observers.iter().enumerate() {
            obstacles.push(circle(PLAYER_ID_BASE + i as u64, ObstacleKind::Skater, o.position, o.velocity, SKATER_SIZE, 1.8));
        }
        // Other NPC skaters (their last sample).
        let mut list: Vec<_> = npcs.iter().filter_map(|n| n.replay.last.as_ref().map(|l| (n.npc.id, l.position, l.velocity))).collect();
        list.sort_by_key(|x| x.0);
        for (id, p, v) in list {
            obstacles.push(circle(id.to_u64(), ObstacleKind::Skater, p, v, SKATER_SIZE, 1.8));
        }
    }
    track.previous = seen;
    track.tick = tick;
    let lines = super::npc_skaters::npc_lines(state);
    npcs.sort_by_key(|n| n.npc.id);
    for entity in npcs.iter_mut() {
        let (npc, replay) = (&entity.npc, &mut entity.replay);
        let a = &mut replay.avoid;
        a.previous_offset = a.offset;
        let (Some(sample), Some(line)) = (replay.last.clone(), lines.get(&replay.cursor.line)) else { continue };
        // The node advance runs first (`sub_8246D3C0`): a new line or the line's end resets the sub-mode
        // (`sub_82468AC8` writes 0 to +568; b70, main checked).
        if a.controller_line != Some(replay.cursor.line) || replay.cursor.finished {
            a.controller.sub_mode = skate_core::living_world::ai_controller::sub_mode::NORMAL;
            a.controller_line = Some(replay.cursor.line);
        }
        let Some(node) = line.nodes.get(replay.cursor.node as usize) else {
            warn!("NPC_AVOID #{} cursor node {} is past its line's {} nodes; skipped this tick", npc.id.serial, replay.cursor.node, line.nodes.len());
            continue;
        };
        let riding = !replay.cursor.finished && node.flags & (node_flags::AIRBORNE | node_flags::OFF_BOARD) == 0 && replay.cursor.current_trick() < 0;
        let me_id = npc.id.to_u64();
        let near: Vec<Obstacle> = obstacles.iter().filter(|o| o.id != me_id).copied().collect();
        let speed = length(sample.velocity);
        let forward = if speed > 1e-3 { sample.velocity.map(|c| c / speed) } else { [0.0, 0.0, 1.0] };
        let here = project_on_line(line, replay.cursor.node, sample.position, 1.0);
        let width = here.map_or(1.0, |p| if a.offset < 0.0 { p.width_left } else { p.width_right });
        // The skater's actual velocity (retail reads the skater object's velocity, vtable +16; b112): how far its
        // sample moved this tick. The recorded velocity stays non-zero while a speed cap holds the cursor, so the
        // closing rate never fell to 0 and a cap of 0 behind a standing ped held for good. A jump (respawn,
        // teleport, line switch) falls back to the recorded velocity for that tick.
        let actual = match a.moved_from {
            Some(p) if dt > 0.0 && length(sub(sample.position, p)) < MAX_TICK_MOVE => sub(sample.position, p).map(|c| c / dt),
            _ => sample.velocity,
        };
        a.moved_from = Some(sample.position);
        let me = AvoidSelf {
            position: (Vec3::from_array(sample.position) + Vec3::from_array(right_of(forward)) * a.offset).to_array(),
            velocity: actual,
            forward,
            lateral: if width > 1e-4 { a.offset / width } else { 0.0 },
        };
        let from = replay.cursor.node;
        let out = if riding { avoid::evaluate(&s, &mut a.state, &me, &near, &|p| project_on_line(line, from, p, s.radius_skater)) } else { AvoidOutput { own_speed: speed, cap: f32::MAX, ..Default::default() } };
        // Speed: hold the cursor back (cap) or let it catch up (floor), on the ground only.
        // Mode 6 enters sub-mode 4 (`sub_8246D560`), which skips the speed shape (`sub_82470830`).
        if out.mode == AvoidMode::LowProp {
            skate_core::living_world::ai_controller::enter_low_prop(&mut a.controller);
        }
        if riding && speed > 1e-3 && !a.low_prop() {
            let rate = out.shape_speed(speed, false) / speed;
            a.lag = (a.lag + (1.0 - rate) * super::npc_skaters::FRAMES_PER_TICK as f32).max(0.0);
        }
        // Steering: towards the gap (mode 3) or the skitch entry (mode 4, `+5976`; b65), back to the line otherwise.
        let goal = match out.mode {
            AvoidMode::Steer | AvoidMode::Skitch if riding => (if out.mode == AvoidMode::Skitch { out.skitch_target } else { out.steer_target })
                .and_then(|id| out.entries.iter().find(|e| e.id == id))
                .and_then(|e| e.path)
                .map_or(0.0, |p| out.lateral * if out.lateral < 0.0 { p.width_left } else { p.width_right }),
            _ => 0.0,
        };
        let step = LATERAL_STEP * super::npc_skaters::FRAMES_PER_TICK as f32;
        a.offset += (goal - a.offset).clamp(-step, step);
        if out.mode != a.last.mode {
            let target = out.steer_target.and_then(|id| out.entries.iter().find(|e| e.id == id));
            info!(
                "NPC_AVOID #{} {} mode {} -> {} target {} cap {} floor {} lateral {:.2} lag {:.1} entries {} tick {tick}",
                npc.id.serial,
                npc.character,
                a.last.mode.name(),
                out.mode.name(),
                target.map_or("none".to_string(), |e| format!("{:?}#{} ttc {:.2}", e.kind, e.id, e.time_to_contact)),
                if out.cap < f32::MAX { format!("{:.2}", out.cap) } else { "none".into() },
                if out.floor_valid { format!("{:.2}", out.floor) } else { "none".into() },
                out.lateral,
                a.lag,
                out.entries.len()
            );
            events.push(NpcSkaterEvent::Avoid { id: npc.id, mode: out.mode, target: target.map(|e| (e.kind, e.id)) });
        }
        a.last = out;
    }
}
