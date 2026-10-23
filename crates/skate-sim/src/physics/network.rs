//! Physics/transport boundary. Remote dynamic proxies join the existing solve;
//! only the locally owned assembly retains its solved reactions.
//!
//! Single-player part of skate-game `physics/network.rs` (`cb79689`): `Proxies`
//! (always empty here, so `solve.rs` and `skeleton_feedback.rs` stay unchanged),
//! `bounds` and its `xyz` helper, all unchanged. Everything that fills proxies from
//! the network (`skate_net` snapshots, `Schema`, `append`, pose capture) is multiplayer-only
//! and not copied. `append_solid` (PR #52 `618e824`, verbatim) joins the traffic cars to
//! the solve.
use super::*;
use glam::{Mat3, Quat};
use skate_core::physics::{
    assembly::BodySnapshot,
    board_world::BoardWorldVolume,
    rigid_body::{RetailQuaternion, world_inverse_inertia},
    world_contact::ContactPrimitive,
};

fn xyz(v: Vector3) -> [f32; 3] {
    [v.x, v.y, v.z]
}
fn vector(v: [f32; 3]) -> Vector3 {
    Vector3::new(v[0], v[1], v[2])
}

pub(crate) fn bounds(p: ContactPrimitive) -> (Vec3, f32) {
    match p {
        ContactPrimitive::Sphere(s) => (Vec3::from_array(xyz(s.center)), s.radius),
        ContactPrimitive::Capsule {
            center,
            half_length,
            radius,
            ..
        } => (Vec3::from_array(xyz(center)), half_length + radius),
        ContactPrimitive::RoundedBox {
            center,
            half_extents,
            radius,
            ..
        } => (
            Vec3::from_array(xyz(center)),
            Vec3::from_array(xyz(half_extents)).length() + radius,
        ),
        ContactPrimitive::Triangle(t) => {
            let vertices = t.vertices.map(|v| Vec3::from_array(xyz(v)));
            let center = (vertices[0] + vertices[1] + vertices[2]) / 3.;
            (
                center,
                vertices
                    .iter()
                    .map(|v| v.distance(center))
                    .fold(0., f32::max)
                    + t.fatness,
            )
        }
    }
}
#[derive(Default)]
pub(crate) struct Proxies {
    pub bodies: Vec<BodySnapshot>,
    pub volumes: Vec<BoardWorldVolume>,
    /// Solids (the traffic cars) and their collision groups.
    pub solids: Vec<(usize, crate::dynamics::SolidBody)>,
    pub groups: std::collections::BTreeMap<usize, u32>,
    pub actors: std::collections::BTreeMap<usize,(u64,usize)>,
    pub dynamics_before: Vec<(u64, usize, [f32; 3], [f32; 3])>,
    pub dynamics_deltas: Vec<(u64, [f32; 3], [f32; 3], [f32; 3], [f32; 3])>,
    /// Ped parts (group 11) in the solve: id, solve index, linear and angular velocity before. Observed only: the ped
    /// is not moved by the solve; its reaction reads the part's velocity after it (`ped_after`, `sub_82E38FB8` kind 5).
    pub observed: Vec<(u64, usize, [f32; 3], [f32; 3])>,
    /// Each observed part's linear and angular velocity after the last solve.
    pub ped_after: Vec<(u64, [f32; 3], [f32; 3])>,
}
// By hand so `clone_from` copies field by field into the existing buffers: the NPC worker sync clones a proxy set per
// job each tick (`npc_sim`), and the derived `clone_from` (= `*self = source.clone()`) allocated every Vec again
// (b333). Same values either way.
impl Clone for Proxies {
    fn clone(&self) -> Self {
        Self {
            bodies: self.bodies.clone(),
            volumes: self.volumes.clone(),
            solids: self.solids.clone(),
            groups: self.groups.clone(),
            actors: self.actors.clone(),
            dynamics_before: self.dynamics_before.clone(),
            dynamics_deltas: self.dynamics_deltas.clone(),
            observed: self.observed.clone(),
            ped_after: self.ped_after.clone(),
        }
    }
    fn clone_from(&mut self, source: &Self) {
        self.bodies.clone_from(&source.bodies);
        self.volumes.clone_from(&source.volumes);
        self.solids.clone_from(&source.solids);
        self.groups.clone_from(&source.groups);
        self.actors.clone_from(&source.actors);
        self.dynamics_before.clone_from(&source.dynamics_before);
        self.dynamics_deltas.clone_from(&source.dynamics_deltas);
        self.observed.clone_from(&source.observed);
        self.ped_after.clone_from(&source.ped_after);
    }
}

/// Where a proxy set stood before some appends ([`Proxies::mark`]), to drop them again ([`Proxies::truncate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProxyMark {
    bodies: usize,
    volumes: usize,
    solids: usize,
    dynamics_before: usize,
    observed: usize,
    /// The solve index of the first body appended after the mark (keys of `groups` / `actors`).
    first_index: usize,
}

/// skate_vita: the single-player body of skate-game's `multiplayer::prepare` (PR #52
/// `618e824`): the proxies of the last tick go before this tick's are pushed (no remote peers).
pub(crate) fn prepare(physics: &mut GamePhysics) {
    physics.network_active = false;
    physics.network_contacts = 0;
    let mut proxies = std::mem::take(&mut physics.network_proxies);
    proxies.bodies.clear();
    proxies.volumes.clear();
    proxies.solids.clear();proxies.groups.clear();proxies.actors.clear();
    proxies.dynamics_before.clear();proxies.dynamics_deltas.clear();
    physics.network_proxies = proxies;
}

impl Proxies {
    /// The current end, for a solve by `skater`.
    pub fn mark(&self, skater: &SkaterRuntime) -> ProxyMark {
        ProxyMark {
            bodies: self.bodies.len(),
            volumes: self.volumes.len(),
            solids: self.solids.len(),
            dynamics_before: self.dynamics_before.len(),
            observed: self.observed.len(),
            first_index: skater.skeleton.bodies().len() + skater.skeleton_drives.targets.bodies.len() + self.bodies.len(),
        }
    }

    /// Drop everything appended after `mark`.
    pub fn truncate(&mut self, mark: ProxyMark) {
        self.bodies.truncate(mark.bodies);
        self.volumes.truncate(mark.volumes);
        self.solids.truncate(mark.solids);
        self.dynamics_before.truncate(mark.dynamics_before);
        self.observed.truncate(mark.observed);
        self.groups.retain(|&k, _| k < mark.first_index);
        self.actors.retain(|&k, _| k < mark.first_index);
    }

    // skate_vita: `append` (remote peers' bodies from `skate_net` snapshots) is multiplayer: not
    // copied.

    /// One actual compound rigid body, expressed at its COM in its principal
    /// inertia frame. Collider poses remain in world space and are not shells.
    pub fn append_solid(
        &mut self, solid: crate::dynamics::SolidBody, physics: &GamePhysics,
        skater: &SkaterRuntime, locally_owned: bool,
    ) {
        if !solid.center_of_mass.is_finite() || !solid.linvel.is_finite()
            || !solid.angvel.is_finite() || !solid.inertia_rotation.is_finite() { return; }
        let index = skater.skeleton.bodies().len()
            + skater.skeleton_drives.targets.bodies.len() + self.bodies.len();
        let mut body = physics.board.bodies()[0];
        let q = solid.inertia_rotation;
        let q = Quat::from_xyzw(q.x, q.y, q.z, q.w).normalize();
        body.rates.position = vector(solid.center_of_mass.to_array());
        body.rates.orientation = RetailQuaternion { x:q.x, y:q.y, z:q.z, w:q.w };
        body.rates.basis = skate_core::math::Basis3 { columns: Mat3::from_quat(q).to_cols_array_2d() };
        // Ownership controls feedback, not physical mass. A remote moving body
        // participates as a finite-mass shadow; only our actor keeps its reaction.
        body.inertia.inverse_mass = solid.inverse_mass;
        body.inertia.inverse_tensor = vector(solid.inverse_inertia.to_array());
        // A host-owned body's drag and caps belong to its original solver.
        body.inertia.linear_drag = 0.;
        body.inertia.angular_drag = 0.;
        body.inertia.maximum_linear_velocity = 100_000.;
        body.inertia.maximum_angular_velocity = 100_000.;
        body.rates.world_inverse_inertia = world_inverse_inertia(body.rates.basis, body.inertia.inverse_tensor);
        body.rates.linear_velocity = vector(solid.linvel.to_array());
        body.rates.angular_velocity = vector(solid.angvel.to_array());
        body.rates.force_acceleration = Vector3::ZERO;
        body.rates.torque_acceleration = Vector3::ZERO;
        body.state_flags = 4;
        self.groups.insert(index, solid.contact_group);
        if locally_owned && solid.inverse_mass > 0. {
            self.dynamics_before.push((solid.id, index, solid.linvel.to_array(), solid.angvel.to_array()));
        } else if solid.contact_group == crate::living_world::ped_body::PED_GROUP && solid.inverse_mass > 0. {
            self.observed.push((solid.id, index, solid.linvel.to_array(), solid.angvel.to_array()));
        }
        self.bodies.push(body);
        self.solids.push((index, solid));
    }

    pub fn capture_dynamics_reactions(
        &mut self, reactions: &[skate_core::physics::rigid_body::RetailReactionCorrections], dt: f32,
    ) {
        self.dynamics_deltas.clear();
        self.ped_after.clear();
        if !(dt.is_finite() && dt > 0.) { return; }
        for &(id, index, v, w) in &self.observed {
            let Some(r) = reactions.get(skate_core::physics::board_step::ATTACHED_REACTION_BASE + index) else { continue };
            let (dv, dw) = (xyz(r.linear_displacement), xyz(r.angular_displacement));
            self.ped_after.push((id, std::array::from_fn(|i| v[i] + dv[i] / dt), std::array::from_fn(|i| w[i] + dw[i] / dt)));
        }
        for &(id, index, _, _) in &self.dynamics_before {
            let Some(r) = reactions.get(skate_core::physics::board_step::ATTACHED_REACTION_BASE + index) else { continue };
            self.dynamics_deltas.push((id,
                xyz(r.linear_displacement).map(|v| v / dt),
                xyz(r.angular_displacement).map(|v| v / dt),
                xyz(r.position_displacement), xyz(r.orientation_displacement),
            ));
        }
    }
}

#[cfg(test)]
mod ped_after_tests {
    use super::*;

    /// A ped part in the solve is observed, never written back: its velocity after the solve is the velocity before
    /// plus the solver's velocity change (the reaction reads it, retail `sub_82E38FB8` kind 5).
    #[test]
    fn an_observed_ped_part_reports_its_velocity_after_the_solve() {
        let mut proxies = Proxies { observed: vec![(7, 2, [1.0, 0.0, 0.0], [0.0; 3])], ..Default::default() };
        let mut reactions = vec![skate_core::physics::rigid_body::RetailReactionCorrections::default(); skate_core::physics::board_step::ATTACHED_REACTION_BASE + 3];
        reactions[skate_core::physics::board_step::ATTACHED_REACTION_BASE + 2].linear_displacement = Vector3::new(0.1, 0.0, 0.0);
        reactions[skate_core::physics::board_step::ATTACHED_REACTION_BASE + 2].angular_displacement = Vector3::new(0.0, 0.05, 0.0);
        proxies.capture_dynamics_reactions(&reactions, 1.0 / 60.0);
        assert_eq!(proxies.ped_after.len(), 1);
        assert!((proxies.ped_after[0].1[0] - 7.0).abs() < 1e-4 && (proxies.ped_after[0].2[1] - 3.0).abs() < 1e-4, "{:?}", proxies.ped_after);
        assert!(proxies.dynamics_deltas.is_empty(), "nothing to write back");
    }
}

#[cfg(test)]
mod clone_tests {
    use super::*;

    /// The hand-written `clone_from` (buffer reuse, b333) gives the same proxy set as `clone`, also over a longer one.
    #[test]
    fn clone_from_equals_clone() {
        let mut a = Proxies::default();
        a.groups.insert(3, 11);
        a.actors.insert(4, (7, 2));
        a.dynamics_before.push((1, 2, [1.0, 2.0, 3.0], [0.5, -0.0, f32::MIN_POSITIVE]));
        a.dynamics_deltas.push((9, [1.0; 3], [2.0; 3], [3.0; 3], [4.0; 3]));
        a.observed.push((5, 6, [0.1; 3], [0.2; 3]));
        a.ped_after.push((5, [0.3; 3], [0.4; 3]));
        let mut b = Proxies::default();
        b.groups.insert(1, 1);
        b.groups.insert(8, 8);
        b.dynamics_before.extend([(0, 0, [9.0; 3], [9.0; 3]); 4]);
        b.ped_after.extend([(0, [9.0; 3], [9.0; 3]); 3]);
        b.clone_from(&a);
        let c = a.clone();
        let bits = |v: &[(u64, usize, [f32; 3], [f32; 3])]| v.iter().map(|e| (e.0, e.1, e.2.map(f32::to_bits), e.3.map(f32::to_bits))).collect::<Vec<_>>();
        assert_eq!((&b.groups, &b.actors), (&c.groups, &c.actors));
        assert_eq!(bits(&b.dynamics_before), bits(&c.dynamics_before));
        assert_eq!(bits(&b.observed), bits(&c.observed));
        assert_eq!(b.dynamics_deltas.len(), c.dynamics_deltas.len());
        assert_eq!(b.ped_after.iter().map(|e| (e.0, e.1.map(f32::to_bits))).collect::<Vec<_>>(), c.ped_after.iter().map(|e| (e.0, e.1.map(f32::to_bits))).collect::<Vec<_>>());
        assert_eq!((b.bodies.len(), b.volumes.len(), b.solids.len()), (0, 0, 0));
    }
}
