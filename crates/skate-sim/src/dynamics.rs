//! The solid-body boundary of the reference's `skate-dynamics` crate (upstream PR #52 `618e824`,
//! `crates/skate-dynamics/src/solid.rs`): one immutable solid (traffic cars in L1c) joining the
//! native skater solve as an attached body, its contacts against the native volumes computed by
//! parry. `SolidCollider`, `SolidBody`, `SolidContact`, `SolidBody::contact` and `valid_contact`
//! are copied verbatim; the crate's Rapier world (mod bodies, definitions, deformation) is not
//! (no mods on the Vita). skate_vita: `parry3d` directly instead of through `rapier3d`; the
//! [`rapier3d`] module gives the copied code its upstream paths.

/// The upstream paths `skate_dynamics::rapier3d::{prelude, parry}` over `parry3d` (the same
/// parry version and types rapier 0.35.3 re-exports).
pub(crate) mod rapier3d {
    pub(crate) use parry3d as parry;
    pub(crate) mod prelude {
        pub(crate) use parry3d::math::{Pose, Rotation, Vector};
        pub(crate) use parry3d::shape::SharedShape;
    }
}

use rapier3d::parry::query;
use rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};

#[derive(Clone)]
pub struct SolidCollider {
    pub shape: SharedShape,
    pub pose: Pose,
    pub friction: f32,
}

#[derive(Clone)]
pub struct SolidBody {
    pub id: u64,
    pub pose: Pose,
    pub center_of_mass: Vector,
    /// Principal inertia frame, not necessarily the visual/body frame.
    pub inertia_rotation: Rotation,
    pub inverse_mass: f32,
    pub inverse_inertia: Vector,
    pub linvel: Vector,
    pub angvel: Vector,
    pub contact_group: u32,
    pub colliders: Vec<SolidCollider>,
}

#[derive(Clone, Copy, Debug)]
pub struct SolidContact {
    pub point_a: Vector,
    pub point_b: Vector,
    /// Out of B, toward A: positive impulse acts on A along this vector.
    pub normal: Vector,
    pub distance: f32,
    pub time_of_impact: f32,
}

/// skate_vita (performance): the centre and radius of a posed shape's bounds, as [`SolidBody::contact`] computes them,
/// so a caller can compute them once per shape ([`SolidBody::contact_near`]).
#[derive(Clone, Copy, Debug)]
pub struct ShapeSphere {
    pub center: Vector,
    pub radius: f32,
}

impl ShapeSphere {
    pub fn of(shape: &dyn rapier3d::parry::shape::Shape, pose: &Pose) -> Self {
        let bounds = shape.compute_aabb(pose);
        Self { center: (bounds.maxs + bounds.mins) * 0.5, radius: (bounds.maxs - bounds.mins).length() * 0.5 }
    }
}

impl SolidBody {
    /// Exact separation/penetration, followed by a full-step translational AND
    /// rotational sweep. The native solver consumes current-frame witness
    /// points, so the speculative separation stays in its own bias equation.
    #[allow(clippy::too_many_arguments)]
    pub fn contact(
        &self,
        collider: &SolidCollider,
        shape_a: &dyn rapier3d::parry::shape::Shape,
        pose_a: Pose,
        com_a: Vector,
        linear_a: Vector,
        angular_a: Vector,
        dt: f32,
        skin: f32,
    ) -> Option<SolidContact> {
        if !(dt.is_finite() && dt > 0. && skin.is_finite() && skin >= 0.) {
            return None;
        }
        let (a, b) = (ShapeSphere::of(shape_a, &pose_a), ShapeSphere::of(&*collider.shape, &collider.pose));
        self.contact_near(collider, shape_a, pose_a, a, b, com_a, linear_a, angular_a, dt, skin)
    }

    /// skate_vita (performance): [`Self::contact`] with both shapes' [`ShapeSphere`]s already computed (`a` of
    /// `shape_a` at `pose_a`, `b` of the collider); the same result.
    #[allow(clippy::too_many_arguments)]
    pub fn contact_near(
        &self,
        collider: &SolidCollider,
        shape_a: &dyn rapier3d::parry::shape::Shape,
        pose_a: Pose,
        a: ShapeSphere,
        b: ShapeSphere,
        com_a: Vector,
        linear_a: Vector,
        angular_a: Vector,
        dt: f32,
        skin: f32,
    ) -> Option<SolidContact> {
        if !(dt.is_finite() && dt > 0. && skin.is_finite() && skin >= 0.) {
            return None;
        }
        let (radius_a, radius_b, center_a, center_b) = (a.radius, b.radius, a.center, b.center);
        let travel = (linear_a - self.linvel).length() * dt
            + angular_a.length() * dt * ((center_a - com_a).length() + radius_a)
            + self.angvel.length() * dt * ((center_b - self.center_of_mass).length() + radius_b);
        if center_a.distance_squared(center_b) > (radius_a + radius_b + travel + skin).powi(2) {
            return None;
        }
        // skate_vita (L1c, performance, exact): one query with the prediction widened by `travel`
        // instead of upstream's at `skin`. Within `skin` it returns the same contact (the
        // closest of the same parts; the prediction only thresholds each part's result);
        // farther than `skin + travel` the shapes cannot meet within `dt` (`travel` bounds the
        // relative motion of any point, the sphere test above uses it the same way), so the
        // sweep, which then finds no hit, is skipped. Near a car most of the skater's volumes
        // are in that band: the Vita spent 10 ms a tick sweeping them against the car mesh.
        skate_core::profile::count("z_solid_near_n", 1);
        let near = skate_core::profile::scope("z_solid_query");
        let first = query::contact(&pose_a, shape_a, &collider.pose, &*collider.shape, skin + travel);
        drop(near);
        match first {
            Ok(Some(c)) if c.dist <= skin => {
                return valid_contact(SolidContact {
                    point_a: c.point1, point_b: c.point2, normal: c.normal2,
                    distance: c.dist, time_of_impact: 0.,
                });
            }
            Ok(None) => return None,
            _ => {}
        }
        // skate_vita (L1c, Vita deviation, chosen by the user 2026-10-17): the sweep is parry's
        // linear cast at the two bodies' linear velocities instead of upstream's nonlinear one
        // (`cast_shapes_nonlinear`, which also turns both bodies at their angular velocities):
        // 3 us against 46 us on the desktop (Vita: about 1.5 ms each, and brushing a car at speed
        // swept 10-15 volumes in one tick). Only the predicted hit of a volume not yet touching
        // differs, by the turn of the limb within the step; touching contacts are unchanged.
        // Upstream's call: `cast_shapes_nonlinear(&NonlinearRigidMotion::new(pose_a,
        // pose_a.inverse() * com_a, linear_a, angular_a), shape_a, &NonlinearRigidMotion::new(
        // collider.pose, collider.pose.inverse() * self.center_of_mass, self.linvel, self.angvel),
        // &*collider.shape, 0., dt, true)`.
        let _ = (com_a, angular_a);
        skate_core::profile::count("z_solid_sweep_n", 1);
        let sweep = skate_core::profile::scope("z_solid_sweep");
        let options = query::ShapeCastOptions {
            max_time_of_impact: dt,
            target_distance: 0.,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
        };
        let hit = query::cast_shapes(&pose_a, linear_a, shape_a, &collider.pose, self.linvel, &*collider.shape, options).ok();
        drop(sweep);
        let hit = hit??;
        // The swept normal is transformed at the impact orientation; witness
        // points are transformed at the CURRENT pose for the native solver.
        let impact_rotation = Rotation::from_scaled_axis(self.angvel * hit.time_of_impact)
            * collider.pose.rotation;
        let normal = impact_rotation * hit.normal2;
        let point_a = pose_a * hit.witness1;
        let point_b = collider.pose * hit.witness2;
        valid_contact(SolidContact {
            point_a, point_b, normal,
            distance: (point_a - point_b).dot(normal),
            time_of_impact: hit.time_of_impact,
        })
    }
}

fn valid_contact(mut c: SolidContact) -> Option<SolidContact> {
    if !c.point_a.is_finite() || !c.point_b.is_finite() || !c.normal.is_finite()
        || !c.distance.is_finite() || c.normal.length_squared() < 0.5 {
        return None;
    }
    c.normal = c.normal.normalize();
    Some(c)
}

#[cfg(test)]
mod sweep_bench {
    use super::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    use super::*;

    /// L1c measurement: what one sweep of a skater capsule against a car mesh costs.
    #[test]
    #[ignore = "requires the living-world export (SKATE3_ASSET_ROOT)"]
    fn sweep_cost_against_the_taxi() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let dir = root.join("private/living_world");
        let data = crate::living_world::vehicles::parse_vehicle_data(&std::fs::read(dir.join("vehicles.json")).unwrap(), None).unwrap();
        let mesh = data.models["vehicle_taxi01"].collision.clone().unwrap().shape;
        let side = mesh.compute_local_aabb().maxs.x;
        let collider = SolidCollider { shape: mesh, pose: Pose::IDENTITY, friction: 0.5 };
        let body = SolidBody { id: 1, pose: Pose::IDENTITY, center_of_mass: Vector::new(0.0, 0.7, 0.0), inertia_rotation: Rotation::IDENTITY, inverse_mass: 0.0, inverse_inertia: Vector::ZERO, linvel: Vector::new(0.0, 0.0, 8.0), angvel: Vector::ZERO, contact_group: 8, colliders: vec![collider.clone()] };
        let capsule = SharedShape::capsule_y(0.15, 0.08);
        let (mut contacts, mut sweeps, mut hits) = (0, 0, 0);
        let started = std::time::Instant::now();
        let n = 2000;
        for i in 0..n {
            // A forearm capsule beside the car's flank, 2-12 cm off the side, closing at 6 m/s and turning.
            let gap = 0.03 + 0.07 * (i % 10) as f32 / 10.0;
            let pose = Pose::from_parts(Vector::new(side + 0.08 + gap, 0.9, -1.0 + (i % 20) as f32 * 0.1), Rotation::from_rotation_z(0.0));
            let c = body.contact(&collider, &*capsule, pose, pose.translation, Vector::new(-6.0, 0.0, 8.0), Vector::new(3.0, 1.0, 0.0), 1.0 / 60.0, 0.025);
            match c {
                Some(c) if c.time_of_impact == 0.0 => contacts += 1,
                Some(_) => { sweeps += 1; hits += 1 }
                None => sweeps += 1,
            }
        }
        let per = started.elapsed().as_secs_f64() * 1e6 / n as f64;
        eprintln!("{n} contacts: {per:.1} us each; within skin {contacts}, swept {sweeps} (hits {hits})");
        // The linear sweep (no rotation) over the same cases, for comparison.
        let started = std::time::Instant::now();
        let mut linear_hits = 0;
        for i in 0..n {
            let gap = 0.03 + 0.07 * (i % 10) as f32 / 10.0;
            let pose = Pose::from_parts(Vector::new(side + 0.08 + gap, 0.9, -1.0 + (i % 20) as f32 * 0.1), Rotation::from_rotation_z(0.0));
            let options = query::ShapeCastOptions { max_time_of_impact: 1.0 / 60.0, target_distance: 0.0, stop_at_penetration: true, compute_impact_geometry_on_penetration: true };
            if let Ok(Some(_)) = query::cast_shapes(&pose, Vector::new(-6.0, 0.0, 0.0), &*capsule, &collider.pose, Vector::ZERO, &*collider.shape, options) {
                linear_hits += 1;
            }
        }
        eprintln!("linear sweeps: {:.1} us each (hits {linear_hits})", started.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
}
