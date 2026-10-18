//! Physics/world primitive query 8277B720 and moving-volume/triangle dispatch
//! 8277BC58. Geometry lives in typed host-owned records; only the recovered
//! kernels' GP and feature scratch boundaries use native word layouts.
#[path = "box_box_sat.rs"]
pub(super) mod box_box_sat;
#[path = "primitive_geometry.rs"]
pub(super) mod geometry;
#[path = "triangle_box_sat.rs"]
pub(super) mod triangle_box_sat;

use super::{
    MaximumFeature, PrimitiveKind, box_maximum_feature,
    projection::best_separating_direction_unless_above,
    capsule_maximum_feature, find_feature_intersection_prism, prism_math::*, project_direction,
    triangle_maximum_feature,
};
use crate::{
    math::Vector3,
    physics::collision::{
        ContactPair, Triangle, TriangleFixup, WorldContactSettings, fix_up_triangle,
        world_separation_limit,
    },
};
pub use geometry::{ContactPrimitive, transform_triangle_volume, triangle_from_volume};
use geometry::{lanes, pack, xyz};

#[derive(Clone, Copy, Debug)]
pub struct PrimitiveContactManifold {
    /// World primitive B toward moving primitive A, as expected by the solver.
    pub normal: Vector3,
    /// Native prism order is retained, including coincident points.
    pub points: [ContactPair; 16],
    pub count: usize,
}

/// skate_vita: a primitive packed once into its native GP record. `pack` is a
/// pure function of the primitive, so a query that meets the same volume or world
/// triangle many times packs it once (a 192-byte record per call on the Vita).
#[derive(Clone, Copy, Debug)]
pub struct PackedPrimitive {
    pub(super) gp: [u32; 48],
    pub(super) kind: PrimitiveKind,
    pub primitive: ContactPrimitive,
}
impl PackedPrimitive {
    pub fn new(primitive: ContactPrimitive) -> Self {
        let (gp, kind) = pack(primitive);
        Self {
            gp,
            kind,
            primitive,
        }
    }
}

/// Full moving sphere/capsule/triangle/rounded-box query against a world
/// triangle. The caller retains native world-triangle then moving-volume order.
pub fn primitive_triangle_world_contacts(
    primitive: ContactPrimitive,
    triangle: Triangle,
    velocity: Vector3,
    settings: WorldContactSettings,
) -> Option<PrimitiveContactManifold> {
    primitive_triangle_world_contacts_packed(
        &PackedPrimitive::new(primitive),
        &PackedPrimitive::new(ContactPrimitive::Triangle(triangle)),
        triangle,
        velocity,
        settings,
    )
}

/// `primitive_triangle_world_contacts` with both shapes already packed
/// (`world` = `PackedPrimitive::new(ContactPrimitive::Triangle(triangle))`).
pub fn primitive_triangle_world_contacts_packed(
    moving: &PackedPrimitive,
    world: &PackedPrimitive,
    triangle: Triangle,
    velocity: Vector3,
    settings: WorldContactSettings,
) -> Option<PrimitiveContactManifold> {
    #[cfg(feature = "kernel-bench")]
    super::kernel_bench::capture_world(moving.primitive, triangle, velocity, settings);
    let limit = world_separation_limit(
        velocity,
        triangle.feature.normal,
        settings.volume_padding,
        settings.maximum_separating_distance,
    );
    let (a, a_kind) = (&moving.gp, moving.kind);
    let (b, b_kind) = (&world.gp, world.kind);
    let (separation, normal) = if matches!(a_kind, PrimitiveKind::Box) {
        // Reversed specialized entry82ACE968 calls triangle/box then flips
        // every sign bit. Native pair direction always points A toward B.
        let (separation, normal) = triangle_box_sat::triangle_box(b, a);
        (separation, normal.map(|v| v ^ 0x8000_0000))
    } else {
        // skate_vita: contact_points rejects a separation above this same
        // threshold first, so the search may stop once it is exceeded
        // (projection.rs, best_separating_direction_unless_above); same outcome.
        let reject_above = (f32::from_bits(b[32]) + limit) + f32::from_bits(a[32]);
        best_separating_direction_unless_above(a, a_kind, b, b_kind, reject_above)?
    };
    contact_points(
        a,
        a_kind,
        b,
        b_kind,
        triangle,
        f32::from_bits(separation[0]),
        normal,
        limit,
        settings,
    )
}

pub(super) fn contact_points(
    a: &[u32; 48],
    a_kind: PrimitiveKind,
    b: &[u32; 48],
    b_kind: PrimitiveKind,
    triangle: Triangle,
    separation: f32,
    initial_normal: [u32; 4],
    limit: f32,
    settings: WorldContactSettings,
) -> Option<PrimitiveContactManifold> {
    let fat_a = f32::from_bits(a[32]);
    let fat_b = f32::from_bits(b[32]);
    if separation > (fat_b + limit) + fat_a {
        return None;
    }
    let mut feature_a = [0; 144];
    maximum(a, a_kind, 1, initial_normal, &mut feature_a);
    let mut feature_b = [0; 144];
    maximum(b, b_kind, 0, initial_normal.map(|v| v ^ 0x8000_0000), &mut feature_b);
    let mut prism = [0; 136];
    if find_feature_intersection_prism(&mut prism, &mut feature_a, &mut feature_b, initial_normal)
        == 0
    {
        return None;
    }
    let count = prism[132] as usize;
    let mut normal = initial_normal.map(f32::from_bits);
    if count == 1 {
        let delta = sub(load(&prism, 64), load(&prism, 0));
        let squared = dot(delta, delta);
        let inverse = inverse_length(squared);
        let length = if squared == 0.0 {
            0.0
        } else {
            squared * inverse
        };
        // This world-query gate compares length. The general Volume query
        // has a different squared-distance gate and publication sequence.
        if length > f32::from_bits(0x3400_0000) {
            prism[128..132].copy_from_slice(&scale(delta, inverse).map(f32::to_bits));
            prism[133] = 1;
        }
    }
    if prism[133] != 0 {
        let direction: [u32; 4] = prism[128..132].try_into().unwrap();
        let mut ai = [0; 12];
        let mut bi = [0; 12];
        project_direction(a, a_kind, direction, &mut ai);
        project_direction(b, b_kind, direction, &mut bi);
        let forward = f32::from_bits(ai[0]) - f32::from_bits(bi[4]);
        let reverse = f32::from_bits(bi[0]) - f32::from_bits(ai[4]);
        let flip = forward > reverse;
        let refined = if flip { forward } else { reverse };
        if refined >= separation {
            normal = direction.map(|v| f32::from_bits(if flip { v ^ 0x8000_0000 } else { v }));
        }
    }
    let mut points = [ContactPair {
        a: Vector3::ZERO,
        b: Vector3::ZERO,
    }; 16];
    for (i, point) in points[..count].iter_mut().enumerate() {
        point.a = xyz(load(&prism, 4 * i));
        point.b = xyz(load(&prism, 64 + 4 * i));
    }
    let mut pair_normal = xyz(normal);
    if !fix_up_triangle(
        triangle.feature,
        &mut pair_normal,
        &mut points[..count],
        TriangleFixup {
            reverse: true,
            edge_cos_bend_normal_threshold: settings.edge_cos_bend_normal_threshold,
            convexity_epsilon: settings.convexity_epsilon,
            is_object: settings.is_object,
        },
    ) {
        return None;
    }
    let normal = lanes(pair_normal, 0.0);
    let a_offset = scale(normal, fat_a);
    let b_offset = scale(normal, fat_b);
    for point in &mut points[..count] {
        // Separate multiply and add/subtract in8277BA50..BA60, after fixup.
        let a = lanes(point.a, 1.0);
        point.a = xyz(std::array::from_fn(|i| a[i] + a_offset[i]));
        point.b = xyz(sub(lanes(point.b, 1.0), b_offset));
    }
    Some(PrimitiveContactManifold {
        normal: xyz(normal.map(|v| -v)),
        points,
        count,
    })
}

/// skate_vita: fills the caller's zeroed record instead of returning it by
/// value (a 576-byte copy per feature on the Vita); same writes.
pub(super) fn maximum(
    gp: &[u32; 48],
    kind: PrimitiveKind,
    mode: u32,
    direction: [u32; 4],
    feature: &mut MaximumFeature,
) {
    match kind {
        PrimitiveKind::Sphere => {
            // Complete sphere callback82ADD7E0: center, point type, header0.
            feature[136..140].copy_from_slice(&gp[..4]);
            feature[140] = 0;
            feature[0] = 0;
        }
        PrimitiveKind::Capsule => {
            capsule_maximum_feature(gp, direction, feature, &mut [0; 16])
        }
        PrimitiveKind::Triangle => triangle_maximum_feature(gp, mode, direction, feature),
        PrimitiveKind::Box => box_maximum_feature(gp, mode, direction, feature, [0; 4]),
    }
}
