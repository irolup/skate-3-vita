use crate::madd::Madd as _;
use crate::math::Vector3 as V;
use crate::physics::reciprocal_sqrt::estimate;

pub(super) fn dot(a: V, b: V) -> f32 {
    crate::physics::native_arithmetic::dot3([a.x, a.y, a.z, 0.0], [b.x, b.y, b.z, 0.0])
}
pub(super) fn cross(a: V, b: V) -> V {
    V::new(
        (-a.z).madd(b.y, a.y * b.z),
        (-a.x).madd(b.z, a.z * b.x),
        (-a.y).madd(b.x, a.x * b.y),
    )
}
pub(super) fn scale(a: V, b: f32) -> V {
    V::new(a.x * b, a.y * b, a.z * b)
}
pub(super) fn neg(a: V) -> V {
    V::new(-a.x, -a.y, -a.z)
}
pub(super) fn sub(a: V, b: V) -> V {
    V::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
pub(super) fn madd(a: V, b: f32, c: V) -> V {
    V::new(
        a.x.madd(b, c.x),
        a.y.madd(b, c.y),
        a.z.madd(b, c.z),
    )
}
pub(super) fn inverse_length_squared(value: f32, refinements: u32) -> f32 {
    let mut r = estimate(value);
    for _ in 0..refinements {
        r = (r * 0.5).madd((-value).madd(r * r, 1.0), r);
    }
    r
}
pub(super) fn normalize(a: V, refinements: u32) -> V {
    scale(a, inverse_length_squared(dot(a, a), refinements))
}
