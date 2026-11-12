//! skate-game's mod hooks the copied physics calls (upstream PR #52). No mods run on the Vita:
//! the stand-ins return what upstream returns with none loaded.

pub(crate) mod player_physics {
    use crate::physics::SkaterRuntime;

    /// Upstream applies the mods' per-part overrides and returns their undo; with no mod, none.
    pub(crate) struct PartRestore;

    impl PartRestore {
        pub(crate) fn restore(self, _skater: &mut SkaterRuntime) {}
    }

    pub(crate) fn apply_parts(_skater: &mut SkaterRuntime) -> PartRestore {
        PartRestore
    }
}

/// skate_vita: upstream `modding/bridge.rs`' `MovingQueries` only (the cars as on-foot ground, `BipedWorld::vehicles`);
/// the mod bodies' side of the bridge is not here (no mods on the Vita).
pub(crate) mod bridge {
    use crate::dynamics::{collider_triangles, sweep_sphere, SolidBody};
    use glam::{Mat4, Quat, Vec3};
    use skate_core::{
        math::Vector3,
        physics::{
            board_world::{ExternalLineHit, ExternalQueries, WorldLineHit},
            triangle_query::TriangleLineHit,
        },
    };

    fn xyz(v: Vector3) -> [f32; 3] { [v.x, v.y, v.z] }
    fn native(v: [f32; 3]) -> Vector3 { Vector3::new(v[0], v[1], v[2]) }

    pub(crate) struct MovingQueries(pub Vec<SolidBody>);

    impl ExternalQueries for MovingQueries {
        fn line(&self, start: Vector3, end: Vector3, radius: f32) -> Option<ExternalLineHit> {
            let (id, c) = sweep_sphere(&self.0, xyz(start), xyz(end), radius)?;
            let body = self.0.iter().find(|b| b.id == id)?;
            let q = body.pose.rotation;
            let transform = Mat4::from_rotation_translation(
                Quat::from_xyzw(q.x, q.y, q.z, q.w), Vec3::from_array(body.pose.translation.to_array()));
            Some(ExternalLineHit {
                hit: WorldLineHit { geometry: TriangleLineHit {
                    position: native(c.point_b.to_array()), normal: native(c.normal.to_array()),
                    fraction: c.time_of_impact, volume_parameter: [0.; 3],
                }, tag: 0 }, surface: 0, geometry_id: 0x8000_0000 | (id as u32 & 0x7fff_ffff),
                frame: transform.to_cols_array_2d(),
            })
        }

        fn nearby(&self, center: Vector3, radius: f32) -> Vec<[Vector3; 3]> {
            let center = Vec3::from_array(xyz(center));
            if !center.is_finite() || !radius.is_finite() || radius < 0. { return Vec::new(); }
            let mut triangles = Vec::new();
            for body in &self.0 {
                for collider in &body.colliders {
                    let aabb = collider.shape.compute_aabb(&collider.pose);
                    let min = Vec3::from_array(aabb.mins.to_array());
                    let max = Vec3::from_array(aabb.maxs.to_array());
                    if center.distance_squared(center.clamp(min, max)) > radius * radius { continue; }
                    for t in collider_triangles(collider) {
                        let verts = t.map(Vec3::from_array);
                        let min = verts[0].min(verts[1]).min(verts[2]);
                        let max = verts[0].max(verts[1]).max(verts[2]);
                        if center.distance_squared(center.clamp(min, max)) <= radius * radius {
                            triangles.push(t.map(native));
                            if triangles.len() >= 64 { return triangles; }
                        }
                    }
                }
            }
            triangles
        }
    }
}
