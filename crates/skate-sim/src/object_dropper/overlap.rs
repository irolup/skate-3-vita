//! The Collision option (Quick Menu > Options; retail `82889940` passes it to the piece's placement check, terrain
//! `8287CF00`): on, a held item that goes into another item turns red and cannot be placed; off, items may overlap and
//! merge into custom shapes (owner, from the game's description).
//!
//! NOT RETAIL YET: retail's check (its collision layers and overlap rules) is not decoded. Ours: the items' boxes
//! first (their catalogue bounds, posed), then their collision triangles where the boxes meet; triangles that only
//! touch or lie flat against each other (a snapped piece, two pieces side by side) are not an overlap, a crossing
//! deeper than [`DEPTH`] is; and an item whose middle is inside the other's shape (a ray up from it crosses the
//! other's surface an odd number of times: pieces are open underneath) is in it.
use glam::{Mat3, Vec3};

/// How far one triangle must cross another's plane, and their crossing run along the line both share, to count.
pub const DEPTH: f32 = 0.02;
/// Triangle pairs tested at most for one item pair (beyond, the pair is taken as clear).
const MAX_PAIRS: usize = 200_000;

/// A posed item: its bounds (template space, scaled) and pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    pub lo: Vec3,
    pub hi: Vec3,
    pub position: Vec3,
    pub rotation: glam::Quat,
}

impl Posed {
    fn centre_axes(&self) -> (Vec3, Mat3, Vec3) {
        let half = (self.hi - self.lo) * 0.5;
        (self.position + self.rotation * (self.lo + half), Mat3::from_quat(self.rotation), half)
    }

    /// The world box around it (min, max).
    pub fn aabb(&self) -> (Vec3, Vec3) {
        let (c, m, h) = self.centre_axes();
        let e = m.x_axis.abs() * h.x + m.y_axis.abs() * h.y + m.z_axis.abs() * h.z;
        (c - e, c + e)
    }
}

/// Whether two posed boxes go into each other by more than [`DEPTH`] (separating axes: 3 + 3 face axes, 9 edge axes).
pub fn boxes_overlap(a: &Posed, b: &Posed) -> bool {
    let (ca, ma, ha) = a.centre_axes();
    let (cb, mb, hb) = b.centre_axes();
    let (axa, axb) = ([ma.x_axis, ma.y_axis, ma.z_axis], [mb.x_axis, mb.y_axis, mb.z_axis]);
    let d = cb - ca;
    let radius = |axes: &[Vec3; 3], h: Vec3, n: Vec3| axes[0].dot(n).abs() * h.x + axes[1].dot(n).abs() * h.y + axes[2].dot(n).abs() * h.z;
    let mut tests: Vec<Vec3> = axa.iter().chain(axb.iter()).copied().collect();
    for x in &axa {
        for y in &axb {
            if let Some(n) = x.cross(*y).try_normalize() {
                tests.push(n);
            }
        }
    }
    tests.into_iter().all(|n| d.dot(n).abs() < radius(&axa, ha, n) + radius(&axb, hb, n) - DEPTH)
}

/// The signed distances of `t`'s corners from the plane through `p` with unit normal `n`.
fn distances(t: &[Vec3; 3], n: Vec3, p: Vec3) -> [f32; 3] {
    t.map(|v| n.dot(v - p))
}

/// Where a triangle crossing a plane (its corners' signed distances `d`) meets it, projected on `axis`: the run.
fn run(t: &[Vec3; 3], d: [f32; 3], axis: Vec3) -> Option<(f32, f32)> {
    let mut points = Vec::with_capacity(2);
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        if (d[i] < 0.0) != (d[j] < 0.0) {
            let s = d[i] / (d[i] - d[j]);
            points.push(axis.dot(t[i] + (t[j] - t[i]) * s));
        }
    }
    let lo = points.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = points.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    (points.len() >= 2).then_some((lo, hi))
}

/// Whether two triangles cross (Möller's interval test): each must cross the other's plane by more than [`DEPTH`]
/// on both sides, and their runs along the shared line overlap by more than [`DEPTH`]. Coplanar or touching
/// triangles do not.
pub fn triangles_cross(a: &[Vec3; 3], b: &[Vec3; 3]) -> bool {
    let (Some(na), Some(nb)) = ((a[1] - a[0]).cross(a[2] - a[0]).try_normalize(), (b[1] - b[0]).cross(b[2] - b[0]).try_normalize()) else {
        return false;
    };
    let strictly = |d: [f32; 3]| d.iter().any(|v| *v > DEPTH) && d.iter().any(|v| *v < -DEPTH);
    let db = distances(b, na, a[0]);
    if !strictly(db) {
        return false;
    }
    let da = distances(a, nb, b[0]);
    if !strictly(da) {
        return false;
    }
    let Some(axis) = na.cross(nb).try_normalize() else { return false };
    let (Some((a0, a1)), Some((b0, b1))) = (run(a, da, axis), run(b, db, axis)) else { return false };
    a1.min(b1) - a0.max(b0) > DEPTH
}

/// Whether two items' collision triangles (each in its own template space, posed by `a` / `b`) cross where their
/// boxes meet.
pub fn shapes_overlap(a: &Posed, a_tris: &[[Vec3; 3]], b: &Posed, b_tris: &[[Vec3; 3]]) -> bool {
    let ((alo, ahi), (blo, bhi)) = (a.aabb(), b.aabb());
    let (lo, hi) = (alo.max(blo) - Vec3::splat(DEPTH), ahi.min(bhi) + Vec3::splat(DEPTH));
    if lo.cmpgt(hi).any() {
        return false;
    }
    let within = |pose: &Posed, tris: &[[Vec3; 3]]| -> Vec<[Vec3; 3]> {
        tris.iter()
            .map(|t| t.map(|v| pose.position + pose.rotation * v))
            .filter(|t| {
                let (tlo, thi) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
                !(tlo.cmpgt(hi).any() || thi.cmplt(lo).any())
            })
            .collect()
    };
    let (ta, tb) = (within(a, a_tris), within(b, b_tris));
    if ta.len() * tb.len() > MAX_PAIRS {
        tracing::warn!("SKATE_DROPPER: overlap check skipped ({} x {} triangles)", ta.len(), tb.len());
        return false;
    }
    ta.iter().any(|x| tb.iter().any(|y| triangles_cross(x, y))) || inside(a, b, b_tris) || inside(b, a, a_tris)
}

/// Whether `a`'s middle is inside `b`'s shape: within `b`'s box, a ray up (`b`'s +y) crosses its triangles an odd
/// number of times.
fn inside(a: &Posed, b: &Posed, b_tris: &[[Vec3; 3]]) -> bool {
    let (centre, ..) = a.centre_axes();
    let local = b.rotation.inverse() * (centre - b.position);
    if local.cmplt(b.lo + Vec3::splat(DEPTH)).any() || local.cmpgt(b.hi - Vec3::splat(DEPTH)).any() {
        return false;
    }
    let hits = b_tris.iter().filter(|t| up_hits(local, t)).count();
    hits % 2 == 1
}

/// Whether the ray up (+y) from `p` crosses triangle `t` (in the same frame) above it.
fn up_hits(p: Vec3, t: &[Vec3; 3]) -> bool {
    // The triangle seen from above: `p` inside it (barycentric), the plane above `p`.
    let (a, b, c) = (t[0], t[1], t[2]);
    let area = (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z);
    if area.abs() < 1e-9 {
        return false;
    }
    let u = ((b.x - p.x) * (c.z - p.z) - (c.x - p.x) * (b.z - p.z)) / area;
    let v = ((c.x - p.x) * (a.z - p.z) - (a.x - p.x) * (c.z - p.z)) / area;
    let w = 1.0 - u - v;
    if u < 0.0 || v < 0.0 || w < 0.0 {
        return false;
    }
    u * a.y + v * b.y + w * c.y > p.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    /// A closed unit-ish box's 12 triangles from `lo` to `hi`.
    fn cube(lo: Vec3, hi: Vec3) -> Vec<[Vec3; 3]> {
        let c = |i: usize| Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
        let faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        faces.iter().flat_map(|f| [[c(f[0]), c(f[1]), c(f[2])], [c(f[0]), c(f[2]), c(f[3])]]).collect()
    }

    fn posed(at: Vec3) -> Posed {
        Posed { lo: Vec3::ZERO, hi: Vec3::new(2.0, 1.0, 2.0), position: at, rotation: Quat::IDENTITY }
    }

    #[test]
    fn side_by_side_and_stacked_pieces_are_clear_and_one_inside_another_is_not() {
        let tris = cube(Vec3::ZERO, Vec3::new(2.0, 1.0, 2.0));
        let a = posed(Vec3::ZERO);
        // Flush side by side, stacked on top, and apart: clear.
        for at in [Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(5.0, 0.0, 0.0)] {
            let b = posed(at);
            assert!(!(boxes_overlap(&a, &b) && shapes_overlap(&a, &tris, &b, &tris)), "{at}");
        }
        // Half way into it: an overlap.
        let b = posed(Vec3::new(1.0, 0.0, 0.5));
        assert!(boxes_overlap(&a, &b) && shapes_overlap(&a, &tris, &b, &tris));
        // A turned piece through it.
        let b = Posed { rotation: Quat::from_rotation_y(0.7), ..posed(Vec3::new(0.5, 0.2, 0.5)) };
        assert!(boxes_overlap(&a, &b) && shapes_overlap(&a, &tris, &b, &tris));
    }

    #[test]
    fn a_rail_above_a_ramp_slope_is_clear_though_their_boxes_meet() {
        // A ramp: one slope from (0, 0) up to (4, 2) along x, 2 m wide; a thin rail 0.3 m above the slope's middle.
        let ramp = vec![[Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 2.0, 0.0), Vec3::new(4.0, 2.0, 2.0)], [Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 2.0, 2.0), Vec3::new(0.0, 0.0, 2.0)]];
        let ramp_pose = Posed { lo: Vec3::ZERO, hi: Vec3::new(4.0, 2.0, 2.0), position: Vec3::ZERO, rotation: Quat::IDENTITY };
        let rail = cube(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.1, 0.1));
        let rail_pose = Posed { lo: Vec3::ZERO, hi: Vec3::new(1.0, 0.1, 0.1), position: Vec3::new(1.5, 1.3, 1.0), rotation: Quat::IDENTITY };
        assert!(boxes_overlap(&ramp_pose, &rail_pose), "the boxes meet");
        assert!(!shapes_overlap(&ramp_pose, &ramp, &rail_pose, &rail), "the surfaces do not");
        // Through the slope, and wholly under it (inside the ramp): an overlap.
        let through = Posed { position: Vec3::new(1.5, 0.8, 1.0), ..rail_pose };
        assert!(shapes_overlap(&ramp_pose, &ramp, &through, &rail));
        let under = Posed { position: Vec3::new(1.5, 0.3, 1.0), ..rail_pose };
        assert!(shapes_overlap(&ramp_pose, &ramp, &under, &rail));
    }
}
