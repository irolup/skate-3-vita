//! skate_vita (L1e): skate-game `living_world/ped_body.rs` (PR #52 `618e824`) without Bevy;
//! `push_ped_proxies` takes the living world's ped state (verbatim body otherwise).
//! Peds as solid bodies in the skater's contact solve (doc 26g, "Peds are solid"; research b138, b144 to b147).
//!
//! Retail: a ped is a body in collision group 11 that the skater's skeleton contact traversal
//! (`sub_82BD4A30`, group switch at `0x82BD5388`) solves against like a car (8) or another skater
//! (5): the solver pushes the two apart, the group-11 force goes to `+4060`, and every contact also
//! feeds the per-region strongest contact the wipeout checks read, so a hard or mid-trick hit into a
//! ped bails the skater through the ordinary thresholds [code, b138 finding 1]. The ped's own
//! reaction (`sub_82E38FB8` kind 5: knock-down / stumble) stays in `peds::skater_hit`.
//!
//! The body (group 11, `sub_82E268C8`) has three capsule parts, measured in the recomp (b146: radius,
//! half length, mass) and posed every frame from the animated skeleton by `sub_82E35838` (b147, main
//! checked the table builder `sub_82E33070`): part 0 at Hips, framed along Hips -> Head; parts 1 / 2
//! halfway between LeftForeArm / LeftHand and RightForeArm / RightHand along the forearm; every part
//! moved ahead by the ped's velocity x 1/60 s. The parts are keyframed (rebuilt from the pose, no
//! write-back found): a finite mass for the solve, no rotation from it.
//! NOT RETAIL YET: part 0's axis is taken as Hips -> Head (retail builds a frame from Hips' third axis
//! and that direction, then turns it 90 degrees; which way is open); peds have no body while in a
//! collision reaction (retail removes bodies A and B together, when is open: user session pending);
//! bodies only within [`PedBodySettings::range`] of an observer (ours: posing costs time). All
//! mod-overridable (`living_world.ped_body`).
use glam::{Mat4, Quat, Vec3};

use super::peds::{PedBody, PedMind, TauntClip};
#[cfg(test)]
use super::peds::PedData;
use skate_core::living_world::peds::Locomotion;

/// Retail ped collision group (`sub_82BD4A30`: 5 skater, 8 vehicle, 11 pedestrian).
pub(crate) const PED_GROUP: u32 = 11;
/// Proxy ids: `PED_PROXY_TAG | ped id << 2 | part` (stable; the multiplayer authority keys).
pub(crate) const PED_PROXY_TAG: u64 = 0x5045_0000_0000_0000;

/// One capsule of the body, between two bones.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PedBodyPart {
    /// Rig bone names (matched ignoring case).
    pub from: String,
    pub to: String,
    /// Where the centre sits from `from` (0) to `to` (1); the axis runs from `from` to `to`.
    pub centre: f32,
    pub radius: f32,
    pub half_length: f32,
    /// kg.
    pub mass: f32,
}

/// `living_world.ped_body` (mod-facing; the default is retail as marked).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PedBodySettings {
    /// Retail on (peds are solid).
    pub enabled: bool,
    pub parts: Vec<PedBodyPart>,
    /// Seconds of ped velocity the parts are moved ahead (retail 1/60).
    pub look_ahead: f32,
    /// Peds farther than this from every observer get no body (m; ours).
    pub range: f32,
}

impl Default for PedBodySettings {
    fn default() -> Self {
        let part = |from: &str, to: &str, centre: f32, radius: f32, half_length: f32, mass: f32| PedBodyPart {
            from: from.into(),
            to: to.into(),
            centre,
            radius,
            half_length,
            mass,
        };
        Self {
            enabled: true,
            parts: vec![
                part("Hips", "Head", 0.0, 0.20, 0.575, 10.0),
                part("LeftForeArm", "LeftHand", 0.5, 0.08, 0.20, 20.0),
                part("RightForeArm", "RightHand", 0.5, 0.08, 0.20, 20.0),
            ],
            look_ahead: 1.0 / 60.0,
            range: 12.0,
        }
    }
}

/// One part's body: a capsule centred between `a` and `b` (world) along `a -> b`, moving at `velocity`.
pub(crate) fn part_proxy(id: u64, index: usize, a: Vec3, b: Vec3, velocity: Vec3, part: &PedBodyPart, look_ahead: f32) -> crate::dynamics::SolidBody {
    use crate::dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    let axis = (b - a).try_normalize().unwrap_or(Vec3::Y);
    let q = Quat::from_rotation_arc(Vec3::Y, axis);
    let centre = a.lerp(b, part.centre) + velocity * look_ahead;
    let rotation = Rotation::from_xyzw(q.x, q.y, q.z, q.w);
    let p = |v: Vec3| Vector::new(v.x, v.y, v.z);
    let pose = Pose::from_parts(p(centre), rotation);
    crate::dynamics::SolidBody {
        id: PED_PROXY_TAG | (id << 2) | index as u64,
        pose,
        center_of_mass: p(centre),
        inertia_rotation: rotation,
        inverse_mass: if part.mass > 0.0 { 1.0 / part.mass } else { 0.0 },
        // Keyframed from the pose: the solve does not turn it.
        inverse_inertia: Vector::new(0.0, 0.0, 0.0),
        linvel: p(velocity),
        angvel: Vector::new(0.0, 0.0, 0.0),
        contact_group: PED_GROUP,
        colliders: vec![crate::dynamics::SolidCollider { shape: SharedShape::capsule_y(part.half_length.max(0.0), part.radius.max(0.01)), pose, friction: 0.5 }],
    }
}

/// A bone's world position: the ped root (position, heading) applied to the model-space global.
pub(crate) fn bone_world(position: Vec3, heading: f32, global: &Mat4) -> Vec3 {
    position + Quat::from_rotation_y(heading) * global.w_axis.truncate()
}

/// Whether the ped is in a collision reaction (the knock-down / stumble from a hit), not a plugin or taunt clip.
fn in_collision(body: &PedBody, mind: Option<&PedMind>) -> bool {
    let clip_reaction = body.taunt == TauntClip::Playing || body.plugin_motion.state == TauntClip::Playing;
    mind.is_some_and(|m| m.brain.colliding) || (body.player.state == Locomotion::Reaction && !clip_reaction)
}

/// Add the near peds' bodies to the skater solve (after the other proxies; rebuilt every tick;
/// `push_ped_proxies`). skate_vita: the body's pose is the one the present step computed for the
/// ped (the same `ped_globals` at the step); a ped not posed this step (out of view) is posed here.
pub(crate) fn push_ped_proxies(
    peds: &super::peds::PedState,
    settings: &super::LivingWorldSettings,
    observers: &[skate_core::living_world::Observer],
    physics: &mut crate::physics::GamePhysics,
    skater: &crate::physics::SkaterRuntime,
) {
    let s = &settings.ped_body;
    let data = &peds.data;
    if !s.enabled || s.parts.is_empty() || data.rig.names.is_empty() {
        return;
    }
    let bone = |name: &str| data.rig.names.iter().position(|n| n.eq_ignore_ascii_case(name));
    let bones: Vec<Option<(usize, usize)>> = s.parts.iter().map(|p| Some((bone(&p.from)?, bone(&p.to)?))).collect();
    let near = |p: Vec3| observers.iter().any(|o| Vec3::from_array(o.position).distance(p) <= s.range);
    let list: Vec<_> = peds.index.values().filter(|e| near(e.body.position) && !in_collision(&e.body, Some(&e.mind))).collect();
    if list.is_empty() {
        return;
    }
    let mut solids = Vec::new();
    for e in list {
        let (ped, body) = (&e.ped, &e.body);
        let posed;
        let globals = match &e.pose {
            Some((_, _, g)) => g,
            None => {
                let Some(g) = super::peds::ped_globals(&data.rig, body, data, 0.0, &[]) else { continue };
                posed = g;
                &posed
            }
        };
        for (index, (part, pair)) in s.parts.iter().zip(&bones).enumerate() {
            let Some((a, b)) = pair.and_then(|(a, b)| Some((globals.get(a)?, globals.get(b)?))) else { continue };
            let (a, b) = (bone_world(body.position, body.heading, a), bone_world(body.position, body.heading, b));
            solids.push(part_proxy(ped.id.to_u64(), index, a, b, body.velocity, part, s.look_ahead));
        }
    }
    let mut proxies = std::mem::take(&mut physics.network_proxies);
    for solid in solids {
        proxies.append_solid(solid, physics, skater, false);
    }
    physics.network_proxies = proxies;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_a_capsule_between_its_bones_in_group_11() {
        let s = PedBodySettings::default();
        let arm = &s.parts[1];
        let b = part_proxy(7, 1, Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.3, 1.2, 0.0), Vec3::new(0.6, 0.0, 0.0), arm, s.look_ahead);
        assert_eq!((b.id, b.contact_group), (PED_PROXY_TAG | 7 << 2 | 1, 11));
        assert!((b.inverse_mass - 1.0 / 20.0).abs() < 1e-6);
        let c = b.pose.translation;
        assert!((c.x - (0.15 + 0.6 / 60.0)).abs() < 1e-5 && (c.y - 1.2).abs() < 1e-5, "midway, moved ahead 1/60 s: {c:?}");
        let capsule = b.colliders[0].shape.as_capsule().unwrap();
        assert!((capsule.radius - 0.08).abs() < 1e-6 && (capsule.half_height() - 0.20).abs() < 1e-5);
        // Along the forearm (+x here): the capsule's segment runs along x.
        let seg = capsule.segment.b - capsule.segment.a;
        let world = b.pose.rotation * seg;
        assert!(world.x.abs() > 0.39 && world.y.abs() < 1e-4, "{world:?}");
        // The default is retail's three parts, 50 kg in total.
        assert_eq!(s.parts.iter().map(|p| p.mass).sum::<f32>(), 50.0);
    }

    /// Data test: the retail parts' bones exist in the exported ped rig, and in its reference pose part 0 stands on
    /// the hips and spans the body (the capsule from about the knees to above the head).
    #[test]
    #[ignore = "requires the private install data (SKATE3_ASSET_ROOT=<repo>\\assets)"]
    fn the_retail_parts_find_their_bones_in_the_ped_rig() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let data = PedData::load(&root);
        assert!(!data.rig.names.is_empty(), "{}", data.status);
        let globals = super::super::peds::reference_globals(&data.rig);
        let at = |name: &str| {
            let i = data.rig.names.iter().position(|n| n.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("no bone {name} in {:?}", data.rig.names));
            bone_world(Vec3::ZERO, 0.0, &globals[i])
        };
        let (hips, head) = (at("Hips"), at("Head"));
        eprintln!("hips {hips:?} head {head:?} left forearm {:?} left hand {:?}", at("LeftForeArm"), at("LeftHand"));
        let part = &PedBodySettings::default().parts[0];
        let (bottom, top) = (hips.y - part.half_length - part.radius, hips.y + part.half_length + part.radius);
        assert!((0.7..1.2).contains(&hips.y) && head.y > hips.y + 0.4, "hips {hips:?} head {head:?}");
        assert!(bottom < 0.4 && top > head.y - 0.2, "part 0 spans {bottom:.2}..{top:.2} m");
    }
}
