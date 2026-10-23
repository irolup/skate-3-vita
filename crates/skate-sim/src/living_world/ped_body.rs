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
//! write-back found): a finite mass for the solve; it turns in the solve with retail's capsule inertia.
//! NOT RETAIL YET: part 0's axis is taken as Hips -> Head (retail builds a frame from Hips' third axis
//! and that direction, then turns it 90 degrees; which way is open); body A (retail's group-9 movement
//! proxy, b342) is not ported, body B stays in the solve through every reaction, as retail (b203);
//! bodies only within [`PedBodySettings::range`] of an observer (ours: posing costs time). All
//! mod-overridable (`living_world.ped_body`).
use glam::{Mat4, Quat, Vec3};

#[cfg(test)]
use super::peds::PedData;

/// Retail ped collision group (`sub_82BD4A30`: 5 skater, 8 vehicle, 11 pedestrian).
pub(crate) const PED_GROUP: u32 = 11;
/// Proxy ids: `PED_PROXY_TAG | ped serial << 2 | part` (stable; the multiplayer authority keys). The serial, not
/// `LivingWorldId::to_u64`: its kind byte (bit 56) shifted by 2 lands in the tag bits, and the tag test then dropped
/// every real ped part (walker and board ped hits were always empty).
pub(crate) const PED_PROXY_TAG: u64 = 0x5045_0000_0000_0000;
const PED_PROXY_TAG_MASK: u64 = 0xFFFF_0000_0000_0000;

pub(crate) fn proxy_id(serial: u32, part: usize) -> u64 {
    PED_PROXY_TAG | (serial as u64) << 2 | part as u64 & 3
}

/// The ped serial of a ped part proxy id, `None` for any other solid.
pub(crate) fn proxy_serial(id: u64) -> Option<u32> {
    (id & PED_PROXY_TAG_MASK == PED_PROXY_TAG).then_some((id >> 2) as u32)
}

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
    /// Contact friction of the part's collider (static; dynamic is 0.75 of it, restitution 0, `solid_contacts`).
    pub friction: f32,
}

/// Retail 0 [code, b289]: every ped part's contact material is static / dynamic friction and restitution 0.0
/// (`82E268C8` fills the three slots from `0x82165A10` = 0.0, the owner's material vfuncs +120 / +124 leave them;
/// `82E0E4B0` writes them to the part definition +0x80, `82DC4588` copies them into the collision object). Our 0.5
/// before had no source and spun the torso about its long axis past the 3.0 rad/s knock-down gate (b288).
pub(crate) const DEFAULT_PART_FRICTION: f32 = 0.0;

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
    /// The part whose speed after the solve decides the ped's reaction, whichever part was touched: retail reads
    /// only part 0, the torso (`sub_82E38FB8` kind 5, `[parts]+76` with no index; b283).
    pub reaction_part: usize,
    /// Body A, the movement proxy (group 9): the body cars and props meet (b342 section 7). `None` = off.
    pub proxy: Option<PedProxy>,
}

/// Body A (retail `sub_82E38400`, group 9; b146 radius / half length / mass, b342): one upright capsule from the ped
/// root. Retail pairs group 9 with the static map, cars (8) and DMOs (12..14), never with the skater or another ped
/// (the collision-type table `40122C30`, read in the recomp, b342 section 7). Ours: props meet it through the
/// pusher rule skater volumes use (`PropDynamics::push_from_skater`) with its mass, so a shoved bench stops against a
/// ped; cars still push the root through `vehicle_contacts` (retail's kind 2 does it through body A). NOT RETAIL
/// YET: the capsule's height offset (`sub_82E35160`, tuning +0x2C, not decoded; ours stands it on the root) and
/// the root following body A for a ped's own plugin object (kind 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PedProxy {
    pub radius: f32,
    pub half_length: f32,
    /// kg.
    pub mass: f32,
}

impl Default for PedProxy {
    fn default() -> Self {
        Self { radius: 0.3125, half_length: 0.5375, mass: 10.0 }
    }
}

/// Retail group of body A.
pub(crate) const PED_PROXY_GROUP: u32 = 9;
/// Body A's actor id in the prop step: the part ids use bits 0..1, body A sets bit 47 (between the serial and the tag,
/// so [`proxy_serial`] still names the ped).
pub(crate) const PED_BODY_A_BIT: u64 = 1 << 47;

pub(crate) fn body_a_id(serial: u32) -> u64 {
    PED_PROXY_TAG | PED_BODY_A_BIT | (serial as u64) << 2
}

/// Body A as a prop-step volume: an upright capsule standing on the root, moving with the ped.
pub(crate) fn body_a_volume(position: Vec3, velocity: Vec3, p: &PedProxy) -> skate_core::physics::board_world::BoardWorldVolume {
    use skate_core::math::Vector3;
    let v = |a: Vec3| Vector3::new(a.x, a.y, a.z);
    let half = p.half_length.max(0.0);
    let radius = p.radius.max(0.01);
    skate_core::physics::board_world::BoardWorldVolume {
        collision_group: PED_PROXY_GROUP,
        // Not a board: the prop's body-bump cap applies.
        body: skate_core::physics::board_step::CollisionBody::Attached(0),
        primitive: skate_core::physics::world_contact::ContactPrimitive::Capsule { center: v(position + Vec3::Y * (radius + half)), axis: v(Vec3::Y), half_length: half, radius },
        motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: v(velocity), ..Default::default() },
        material: skate_core::physics::contact::RetailContactMaterial { static_friction: 0.0, dynamic_friction: 0.0, restitution: 0.0 },
    }
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
            friction: DEFAULT_PART_FRICTION,
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
            reaction_part: 0,
            proxy: Some(PedProxy::default()),
        }
    }
}

/// The (|v|, |w|) after the solve of ped `serial`'s reaction part, from every ped part in the solve (`after`: proxy
/// id, speeds). `None` when the settings name no such part or it was not in the solve.
pub(crate) fn reaction_part_speed(after: &[(u64, [f32; 2])], serial: u32, s: &PedBodySettings) -> Option<[f32; 2]> {
    if s.reaction_part >= s.parts.len() {
        return None;
    }
    let id = proxy_id(serial, s.reaction_part);
    after.iter().find(|p| p.0 == id).map(|p| p.1)
}

/// One part's body: a capsule centred between `a` and `b` (world) along `a -> b`, moving at `velocity`.
pub(crate) fn part_proxy(serial: u32, index: usize, a: Vec3, b: Vec3, velocity: Vec3, part: &PedBodyPart, look_ahead: f32) -> crate::dynamics::SolidBody {
    use crate::dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    let axis = (b - a).try_normalize().unwrap_or(Vec3::Y);
    let q = Quat::from_rotation_arc(Vec3::Y, axis);
    let centre = a.lerp(b, part.centre) + velocity * look_ahead;
    let rotation = Rotation::from_xyzw(q.x, q.y, q.z, q.w);
    let p = |v: Vec3| Vector::new(v.x, v.y, v.z);
    let pose = Pose::from_parts(p(centre), rotation);
    crate::dynamics::SolidBody {
        id: proxy_id(serial, index),
        pose,
        center_of_mass: p(centre),
        inertia_rotation: rotation,
        inverse_mass: if part.mass > 0.0 { 1.0 / part.mass } else { 0.0 },
        // Posed every frame (not written back), but it turns in the solve like retail's part, whose |w| afterwards
        // can knock the ped down.
        inverse_inertia: inverse_inertia(part),
        linvel: p(velocity),
        angvel: Vector::new(0.0, 0.0, 0.0),
        contact_group: PED_GROUP,
        colliders: vec![crate::dynamics::SolidCollider { shape: SharedShape::capsule_y(part.half_length.max(0.0), part.radius.max(0.01)), pose, friction: part.friction.max(0.0) }],
    }
}

/// A capsule part's inverse inertia in its own frame (axis y): retail `sub_82AE7770` from the shape's per-kg inertia
/// (`sub_82AE7228`, capsule: R = max(r, 0.05 h), along the axis 0.4 R^2, across (1.6 R^3 + 0.75 h R^2 + 4 h^2 R + h^3)
/// / (4 R + 3 h)) times the mass (research b275, matches the PEDPART trace). Massless: no rotation.
pub(crate) fn inverse_inertia(part: &PedBodyPart) -> crate::dynamics::rapier3d::prelude::Vector {
    use crate::dynamics::rapier3d::prelude::Vector;
    let (h, m) = (part.half_length.max(0.0), part.mass);
    let r = part.radius.max(1e-7).max(0.05 * h);
    let along = 0.4 * r * r;
    let across = (1.6 * r * r * r + 0.75 * h * r * r + 4.0 * h * h * r + h * h * h) / (4.0 * r + 3.0 * h);
    if !(m > 0.0) {
        return Vector::new(0.0, 0.0, 0.0);
    }
    Vector::new(1.0 / (across * m), 1.0 / (along * m), 1.0 / (across * m))
}

/// A bone's world position: the ped root (position, heading) applied to the model-space global.
pub(crate) fn bone_world(position: Vec3, heading: f32, global: &Mat4) -> Vec3 {
    position + Quat::from_rotation_y(heading) * global.w_axis.truncate()
}

/// Add the near peds' bodies to the skater solve (after the other proxies; rebuilt every tick;
/// `push_ped_proxies`). The bodies stay through a stumble or knock-down, as retail's do: retail removes a ped's
/// collision bodies only in the collision object's destructor (`sub_82E22D50`, ped despawn; research b203, laptop
/// trace 175036), so a skater cannot roll through a ped that is already reacting. New hits during the reaction are
/// still skipped (`peds.rs`, retail `sub_82E38FB8`). skate_vita: the body's pose is the one the present step computed for the
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
    physics.ped_prop_volumes.clear();
    if !s.enabled {
        return;
    }
    let near = |p: Vec3| observers.iter().any(|o| Vec3::from_array(o.position).distance(p) <= s.range);
    // Body A for the prop step: every near ped, whether or not body B can be posed (b342).
    if let Some(proxy) = s.proxy.as_ref() {
        let mut a: Vec<_> = peds
            .index
            .values()
            .filter(|e| near(e.body.position))
            .map(|e| (body_a_id(e.ped.id.serial), body_a_volume(e.body.position, e.body.velocity, proxy), proxy.mass))
            .collect();
        a.sort_by_key(|x| x.0);
        physics.ped_prop_volumes = a;
    }
    if s.parts.is_empty() || data.rig.names.is_empty() {
        return;
    }
    let bone = |name: &str| data.rig.names.iter().position(|n| n.eq_ignore_ascii_case(name));
    let bones: Vec<Option<(usize, usize)>> = s.parts.iter().map(|p| Some((bone(&p.from)?, bone(&p.to)?))).collect();
    let list: Vec<_> = peds.index.values().filter(|e| near(e.body.position)).collect();
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
            solids.push(part_proxy(ped.id.serial, index, a, b, body.velocity, part, s.look_ahead));
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
    fn body_a_is_an_upright_root_capsule_in_group_9_with_its_own_id() {
        let p = PedProxy::default();
        assert_eq!((p.radius, p.half_length, p.mass), (0.3125, 0.5375, 10.0), "b146 / b342 retail values");
        let v = body_a_volume(Vec3::new(1.0, 2.0, 3.0), Vec3::X, &p);
        assert_eq!(v.collision_group, 9);
        let skate_core::physics::world_contact::ContactPrimitive::Capsule { center, axis, half_length, radius } = v.primitive else { panic!("capsule") };
        assert!((center.y - (2.0 + 0.3125 + 0.5375)).abs() < 1e-5 && axis.y == 1.0 && half_length == 0.5375 && radius == 0.3125);
        // Body A's id never equals a body B part id of any ped, and still names the ped.
        let a = body_a_id(7);
        assert!((0..4).all(|part| a != proxy_id(7, part)) && proxy_serial(a) == Some(7));
        assert!(PedBodySettings::default().proxy.is_some(), "retail on");
    }

    #[test]
    fn the_parts_turn_with_retails_capsule_inertia() {
        // b275: the PEDPART trace's inverse inertia (across, along the axis): torso 0.520984 / 6.25, forearm 2.037273 / 19.53125.
        let parts = PedBodySettings::default().parts;
        for (part, across, along) in [(&parts[0], 0.520984, 6.25), (&parts[1], 2.037273, 19.53125)] {
            let i = inverse_inertia(part);
            assert!((i.x - across).abs() < 1e-4 && (i.z - across).abs() < 1e-4 && (i.y - along).abs() < 1e-3, "{i:?}");
        }
    }

    #[test]
    fn a_part_is_a_capsule_between_its_bones_in_group_11() {
        let s = PedBodySettings::default();
        let arm = &s.parts[1];
        let b = part_proxy(7, 1, Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.3, 1.2, 0.0), Vec3::new(0.6, 0.0, 0.0), arm, s.look_ahead);
        assert_eq!(b.colliders[0].friction, 0.0, "retail ped parts have no friction (b289)");
        assert_eq!((b.id, b.contact_group), (PED_PROXY_TAG | 7 << 2 | 1, 11));
        assert_eq!(proxy_serial(b.id), Some(7));
        // Any serial keeps the tag bits (the kind byte of `to_u64` did not: no ped hit was ever reported).
        assert_eq!(proxy_serial(proxy_id(u32::MAX, 3)), Some(u32::MAX));
        let ped = skate_core::living_world::LivingWorldId { kind: skate_core::living_world::Kind::Pedestrian, serial: 5 };
        assert_eq!(proxy_serial(ped.to_u64() << 2), None, "a kind byte in the id is not a ped proxy");
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

    /// b283: retail's reaction reads the torso (part 0) only, so a fast-spinning forearm touched alone does not decide
    /// it; a mod can name another part, and a part that does not exist gives no reading.
    #[test]
    fn the_reaction_reads_the_torso_whichever_part_was_touched() {
        let mut s = PedBodySettings::default();
        assert_eq!(s.reaction_part, 0);
        let after = [(proxy_id(7, 0), [0.9, 1.2]), (proxy_id(7, 1), [2.0, 9.5]), (proxy_id(8, 0), [5.0, 5.0])];
        assert_eq!(reaction_part_speed(&after, 7, &s), Some([0.9, 1.2]));
        assert_eq!(reaction_part_speed(&after, 9, &s), None, "not in the solve");
        s.reaction_part = 1;
        assert_eq!(reaction_part_speed(&after, 7, &s), Some([2.0, 9.5]));
        s.reaction_part = 3;
        assert_eq!(reaction_part_speed(&after, 7, &s), None, "only three parts");
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
