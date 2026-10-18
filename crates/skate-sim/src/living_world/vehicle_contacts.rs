//! skate_vita (L1e): skate-game `living_world/vehicle_contacts.rs` (PR #52 `618e824`) without
//! Bevy: `ped_vehicle_contacts` takes the living world's peds and traffic; `car_box` and `resolve`
//! verbatim.
//! Traffic cars touching peds (doc 26, "Cars hit peds"; rules and retail evidence in
//! `skate_core::living_world::peds::vehicle_contact`).
//!
//! Retail [code]: a ped's contact with an `IVehicle` body is contact kind 2 (`sub_82E38400`); the
//! ped's callback (`sub_82E38FB8`) only lets the ped's root follow its collision body, which the
//! car's kinematic box has shoved aside. No knock-down, stumble, ragdoll, speech or fade.
//!
//! Here (`FixedUpdate`, after the peds and the cars stepped this tick): every ped against every
//! car's box (GLB bounds, the same box as the car's skater proxy), peds and cars in id order so
//! the result is deterministic; a contact pushes the ped out ([`VehicleContactReaction::Push`],
//! kept on its navmesh), is published as a serialisable [`VehicleContactEvent`] (the planned
//! `sdk.living_world` events and a future host read it) and logged as `VEHICLE_CONTACT` (once per
//! car and ped per second). Rules: [`LivingWorldSettings::ped_vehicle_contact`]
//! (`sdk.world.set_tuning("living_world", {ped_vehicle_contact = {enabled, push}})`); a mod that
//! stops is undone with the settings reset.
//!
//! NOT RETAIL YET: the ped body is a cylinder of the NavPower agent radius / height (retail's
//! Havok ped shape is not decoded), the push is the smallest separation in the ground plane
//! (Havok's penetration recovery is not decoded), the navmesh stands in for the world collision
//! of the pushed body; the car side (hit-by mask, horn, the planner stopping for peds) is V4.

use super::peds::PedBody;
use super::vehicles::TrafficCar;
use crate::transform::Transform;
use glam::Vec3;
use skate_core::living_world::LivingWorldId;
use super::{LivingWorldSettings, PopulationState};
use skate_core::living_world::peds::vehicle_contact::{closing_speed, detect};
use skate_core::living_world::peds::wander::constrain_move;
use skate_core::living_world::peds::{CarBox, PedCylinder, VehicleContactParams, VehicleContactReaction};
use std::collections::BTreeMap;

/// NavPower agent radius / height used without a navmesh [data: the navmesh header's 0.35 / 1.6].
pub(crate) const FALLBACK_PED_RADIUS: f32 = 0.35;
pub(crate) const FALLBACK_PED_HEIGHT: f32 = 1.6;

/// One car touching one ped (serialisable: stable ids as `LivingWorldId::to_u64`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VehicleContactEvent {
    pub tick: u64,
    pub car: u64,
    pub ped: u64,
    /// The car's speed, m/s.
    pub car_speed: f32,
    /// The car's speed into the ped along the contact normal, m/s.
    pub closing: f32,
    /// Ped feet before the push.
    pub position: [f32; 3],
    pub normal: [f32; 2],
    pub depth: f32,
    /// `push` or `reported`.
    pub reaction: String,
}

/// The collision box of a car at its fixed-step pose.
pub(crate) fn car_box(car: &TrafficCar, pose: &Transform) -> CarBox {
    let [lo, hi] = car.bounds;
    let half = [((hi[0] - lo[0]) * 0.5).max(0.1), ((hi[1] - lo[1]) * 0.5).max(0.1), ((hi[2] - lo[2]) * 0.5).max(0.1)];
    let centre = pose.translation + pose.rotation * Vec3::new((hi[0] + lo[0]) * 0.5, (hi[1] + lo[1]) * 0.5, (hi[2] + lo[2]) * 0.5);
    let f = pose.rotation * Vec3::Z;
    CarBox { center: centre.to_array(), forward: [f.x, f.z], half }
}

/// Resolve every ped against every car once (ids ascending). Returns the events, pushed peds
/// already moved.
pub(crate) fn resolve(
    params: &VehicleContactParams,
    tick: u64,
    mesh: Option<&skate_core::living_world::peds::NavMesh>,
    cars: &[(u64, CarBox, Vec3)],
    peds: &mut [(u64, &mut PedBody)],
) -> Vec<VehicleContactEvent> {
    let mut out = Vec::new();
    if !params.enabled || cars.is_empty() {
        return out;
    }
    let (radius, height) = mesh.map_or((FALLBACK_PED_RADIUS, FALLBACK_PED_HEIGHT), |m| (m.agent[1], m.agent[3]));
    let reaction = params.reaction();
    for (ped, body) in peds.iter_mut() {
        for (car, b, velocity) in cars {
            // Broad phase: the box's half diagonal plus the body.
            let reach = (b.half[0] * b.half[0] + b.half[2] * b.half[2]).sqrt() + radius;
            let (dx, dz) = (body.position.x - b.center[0], body.position.z - b.center[2]);
            if dx * dx + dz * dz > reach * reach {
                continue;
            }
            let cyl = PedCylinder { feet: body.position.to_array(), radius, height };
            let Some(c) = detect(b, &cyl) else { continue };
            out.push(VehicleContactEvent {
                tick,
                car: *car,
                ped: *ped,
                car_speed: velocity.length(),
                closing: closing_speed(c.normal, velocity.to_array(), [0.0; 3]),
                position: cyl.feet,
                normal: c.normal,
                depth: c.depth,
                reaction: reaction.name().to_string(),
            });
            if reaction == VehicleContactReaction::Push {
                match mesh {
                    Some(m) => {
                        let (next, poly, _) = constrain_move(m, body.nav.poly, body.position.to_array(), c.pushed);
                        body.position = Vec3::from_array(next);
                        body.nav.poly = poly;
                    }
                    None => body.position = Vec3::from_array(c.pushed),
                }
            }
        }
    }
    out
}

/// Every ped against every car once (`ped_vehicle_contacts`), after the peds and the cars stepped:
/// a pushed ped moves in the ground plane (its drawn feet too). Logs `VEHICLE_CONTACT` once per car
/// and ped per second.
pub(crate) fn ped_vehicle_contacts(
    settings: &LivingWorldSettings,
    state: &mut PopulationState,
    peds: &mut super::peds::PedState,
    traffic: &super::vehicles::TrafficState,
    logged: &mut BTreeMap<(u64, u64), u64>,
) -> Vec<VehicleContactEvent> {
    let _scope = skate_core::profile::scope("z_ped_cars");
    let mut list: Vec<(u64, CarBox, Vec3)> = traffic.index.values().map(|(e, _)| (e.meta.id.to_u64(), car_box(&e.meta, &e.motion.curr), e.motion.velocity)).collect();
    if list.is_empty() || peds.index.is_empty() {
        return Vec::new();
    }
    list.sort_by_key(|(id, ..)| *id);
    let tick = state.world.tick();
    let hz = state.world.clock().hz as u64;
    let mesh = peds.data.nav.clone();
    let mut rows: Vec<&mut super::peds::PedEntity> = peds.index.values_mut().collect();
    rows.sort_by_key(|e| e.ped.id.to_u64());
    let mut bodies: Vec<(u64, &mut PedBody)> = Vec::with_capacity(rows.len());
    let mut shown: Vec<(LivingWorldId, &mut Vec3)> = Vec::with_capacity(rows.len());
    for e in rows {
        let id = e.ped.id;
        bodies.push((id.to_u64(), &mut e.body));
        shown.push((id, &mut e.shown));
    }
    let found = resolve(&settings.ped_vehicle_contact, tick, mesh.as_deref(), &list, &mut bodies);
    if found.is_empty() {
        return found;
    }
    // Retail keeps the root's height: move the drawn ped in the ground plane only.
    for ((_, body), (id, at)) in bodies.iter().zip(shown.iter_mut()) {
        if at.x != body.position.x || at.z != body.position.z {
            at.x = body.position.x;
            at.z = body.position.z;
            state.world.update_position(*id, body.position.to_array());
        }
    }
    logged.retain(|_, t| tick < *t + hz);
    for e in &found {
        if !logged.contains_key(&(e.car, e.ped)) {
            logged.insert((e.car, e.ped), tick);
            debug!(
                "VEHICLE_CONTACT car=#{} ped=#{} speed={:.2} closing={:.2} at=[{:.2}, {:.2}, {:.2}] normal=[{:.2}, {:.2}] depth={:.3} reaction={} tick={}",
                e.car & 0xFFFF_FFFF, e.ped & 0xFFFF_FFFF, e.car_speed, e.closing, e.position[0], e.position[1], e.position[2], e.normal[0], e.normal[1], e.depth, e.reaction, e.tick,
            );
        }
    }
    found
}
