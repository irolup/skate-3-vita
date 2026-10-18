//! skate_vita: copied from upstream PR #52 `618e824` (`physics/solid_contacts.rs`); paths
//! `skate_dynamics::` -> `crate::dynamics::`, Bevy math -> glam.
//! Solid Rapier/native solver boundary. Native bone/board primitives remain
//! native; convex objects are tested with their actual Parry support shapes.
use super::{GamePhysics, SkaterRuntime};
use glam::{Mat3, Quat, Vec3};
use skate_core::{math::Vector3, physics::{
    board_step::{BoardCollision, CollisionBody}, board_world::BoardWorldVolume,
    contact::{RetailContactInput, RetailContactMaterial, combine_contact_materials},
    world_contact::ContactPrimitive,
}};
use crate::dynamics::{SolidBody, rapier3d::{prelude::{SharedShape, Pose, Rotation, Vector}}};

pub(crate) fn vector(v: Vector3) -> Vector { Vector::new(v.x, v.y, v.z) }
fn native(v: Vector) -> Vector3 { Vector3::new(v.x, v.y, v.z) }

pub(crate) fn shape(primitive: ContactPrimitive) -> Option<(SharedShape, Pose)> {
    let (shape, center, rotation) = match primitive {
        ContactPrimitive::Sphere(s) => (SharedShape::ball(s.radius), s.center, Quat::IDENTITY),
        ContactPrimitive::Capsule { center, axis, half_length, radius } => {
            let axis = Vec3::new(axis.x, axis.y, axis.z).try_normalize()?;
            (SharedShape::capsule_y(half_length, radius), center, Quat::from_rotation_arc(Vec3::Y, axis))
        }
        ContactPrimitive::RoundedBox { center, basis, half_extents, radius } => (
            SharedShape::round_cuboid(half_extents.x, half_extents.y, half_extents.z, radius),
            center, Quat::from_mat3(&Mat3::from_cols_array_2d(&basis.columns)).normalize(),
        ),
        ContactPrimitive::Triangle(_) => return None,
    };
    Some((shape, Pose::from_parts(vector(center), Rotation::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w))))
}

pub(crate) fn append(
    contacts: &mut Vec<BoardCollision>, volumes: &[BoardWorldVolume],
    bodies: &[(usize, SolidBody)], physics: &GamePhysics, skater: &SkaterRuntime,
) {
    let dt = physics.settings.step.simulation.time_step;
    for a in volumes {
        let Some((shape_a, pose_a)) = shape(a.primitive) else { continue };
        let native_body = match a.body {
            CollisionBody::Board(id) => physics.board.bodies().get(id.index()),
            CollisionBody::Attached(index) => skater.skeleton.bodies().get(index),
            CollisionBody::StaticWorld => None,
        };
        let Some(native_body) = native_body else { continue };
        for (index, body) in bodies {
            for collider in &body.colliders {
                let Some(c) = body.contact(collider, &*shape_a, pose_a,
                    vector(native_body.rates.position), vector(native_body.rates.linear_velocity),
                    vector(native_body.rates.angular_velocity), dt, 0.025) else { continue };
                let material = combine_contact_materials(a.material, RetailContactMaterial {
                    static_friction: collider.friction,
                    dynamic_friction: collider.friction * 0.75, restitution: 0.,
                });
                contacts.push(BoardCollision {
                    body_a: a.body, body_b: CollisionBody::Attached(*index),
                    contact: RetailContactInput {
                        position_on_a: native(c.point_a), position_on_b: native(c.point_b),
                        normal: native(c.normal), restitution: material.restitution,
                        static_friction: material.static_friction,
                        dynamic_friction: material.dynamic_friction, tag: 0,
                    },
                });
            }
        }
    }
}

/// Last-mile constraint for animation-driven walking. Native walking may write
/// its root without integrating a Rapier body; a full-height capsule sweep keeps
/// that displacement from bypassing the same solid obstacle used by BoardStep.
/// Only mod solids participate. Stock floor and movement algorithms are intact.
fn constrained_position(solids:&[(usize,SolidBody)],start:Vec3,target:Vec3) -> Vec3 {
    use crate::dynamics::rapier3d::parry::query::{self,ShapeCastOptions};
    if solids.is_empty() || !start.is_finite() || !target.is_finite() { return target; }
    const SKIN:f32=0.015;
    let capsule=SharedShape::capsule_y(0.55,0.28);
    let mut position=Vector::from_array(start.to_array());
    let mut remaining=Vector::from_array((target-start).to_array());
    // Deep overlap recovery, including a body spawned around a walking actor.
    // It uses the closed convex volume, not triangle front faces.
    for _ in 0..8 {
        let pose=Pose::from_translation(position);
        let mut deepest:Option<(f32,Vector)>=None;
        for (_,body) in solids {
            for collider in &body.colliders {
                if let Ok(Some(c))=query::contact(&pose,&*capsule,&collider.pose,&*collider.shape,SKIN) {
                    if c.dist<0. && deepest.as_ref().is_none_or(|(d,_)|c.dist<*d) {
                        deepest=Some((c.dist,c.normal2));
                    }
                }
            }
        }
        let Some((distance,normal))=deepest else { break };
        if !normal.is_finite() || !distance.is_finite() { break; }
        position+=normal*(-distance+SKIN);
    }
    for _ in 0..5 {
        if remaining.length_squared()<1e-10 { break; }
        let pose=Pose::from_translation(position);
        let mut nearest:Option<(f32,Vector)>=None;
        for (_,body) in solids {
            for collider in &body.colliders {
                let options=ShapeCastOptions { max_time_of_impact:1.,target_distance:SKIN,
                    stop_at_penetration:true,compute_impact_geometry_on_penetration:true };
                if let Ok(Some(hit))=query::cast_shapes(&pose,remaining,&*capsule,&collider.pose,Vector::ZERO,&*collider.shape,options) {
                    let normal=collider.pose.rotation*hit.normal2;
                    // Contacts behind us cannot block movement away/tangential.
                    if normal.dot(remaining) >= -1e-7 { continue; }
                    if nearest.as_ref().is_none_or(|(t,_)|hit.time_of_impact<*t) {
                        nearest=Some((hit.time_of_impact,normal));
                    }
                }
            }
        }
        let Some((time,normal))=nearest else { position+=remaining; break };
        position+=remaining*time.clamp(0.,1.)+normal*0.001;
        remaining*=1.-time.clamp(0.,1.);
        remaining-=normal*remaining.dot(normal).min(0.);
    }
    Vec3::from_array(position.to_array())
}

pub(super) fn constrain_ground(
    owner:&mut super::biped_ground::Owner,
    result:&mut skate_core::player::offboard::controller::GroundResult,
    previous:[f32;4],solids:&[(usize,SolidBody)],
) {
    let start=Vec3::new(previous[0],previous[1],previous[2]);
    let requested=Vec3::new(result.position[0],result.position[1],result.position[2]);
    let delta=constrained_position(solids,start,requested)-requested;
    if delta.length_squared()<1e-12 || !delta.is_finite() { return; }
    let shift=|v:&mut [f32;4]| { v[0]+=delta.x;v[1]+=delta.y;v[2]+=delta.z; };
    shift(&mut result.position);
    shift(&mut result.physical_frame[3]);
    shift(&mut result.animation_frame[3]);
    let state=&mut owner.controller.state;
    shift(&mut state.position_368);
    shift(&mut state.motion.frame_0[3]);
    shift(&mut state.motion.published_frame_64[3]);
    shift(&mut state.frame_output.frame[3]);
    shift(&mut state.correction_target_592);
    owner.result=Some(*result);
    // Do not replace the actor's velocity with obstacle velocity here: native
    // contact reports must see the real relative impact speed for wipeout.
}

#[cfg(test)]
mod tests {
    use super::*;
    // skate_vita: the reference spawns this box in its Rapier world (`DynamicsWorld::spawn`, a
    // `Shape::Box` at the origin); with no Rapier world here the same solid is built directly.
    fn obstacle() -> Vec<(usize,SolidBody)> {
        vec![(0, SolidBody {
            id: 1, pose: Pose::IDENTITY, center_of_mass: Vector::ZERO, inertia_rotation: Rotation::IDENTITY,
            inverse_mass: 1., inverse_inertia: Vector::ONE, linvel: Vector::ZERO, angvel: Vector::ZERO, contact_group: 0,
            colliders: vec![crate::dynamics::SolidCollider { shape: SharedShape::cuboid(0.5,2.,2.), pose: Pose::IDENTITY, friction: 0.5 }],
        })]
    }
    #[test] fn a_walk_step_cannot_cross_the_whole_object() {
        let end=constrained_position(&obstacle(),Vec3::new(-3.,0.,0.),Vec3::new(3.,0.,0.));
        assert!(end.x < -0.77);
    }
    #[test] fn walking_away_is_not_blocked() {
        let end=constrained_position(&obstacle(),Vec3::new(-0.80,0.,0.),Vec3::new(-2.,0.,0.));
        assert!((end.x+2.).abs()<1e-4);
    }
    #[test] fn deep_inside_start_is_recovered() {
        let end=constrained_position(&obstacle(),Vec3::ZERO,Vec3::ZERO);
        assert!(end.x.abs() > 0.77);
    }
}
