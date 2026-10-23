//! Retail's DMO ground contact reduction (`82779EB0` with `8277A208` / `827798F8`): coplanar contacts of one body pair
//! keep at most four points. Spec and evidence: docs/hails-additions/27-dynamic-props.md#ground-contact-reduction

use crate::math::Vector3;

/// One contact record: `p0` on the moving body, `p1` on the world, `normal` from the world toward the body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactRecord {
    pub p0: Vector3,
    pub p1: Vector3,
    pub normal: Vector3,
}

/// The reduction's values (retail defaults: `820ED800` 0.999, `820D71E8` 0.01, the 50-record job buffer).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReductionRules {
    pub enabled: bool,
    /// Same group when the normals' dot is at least this.
    pub normal_dot: f32,
    /// ... and the nearer of the two point pairs lies within this of the head's plane, m.
    pub plane_distance: f32,
    /// Records reduced together at most (a full buffer is reduced and emitted, nothing is dropped).
    pub chunk: usize,
}

impl Default for ReductionRules {
    fn default() -> Self {
        Self { enabled: true, normal_dot: 0.999, plane_distance: 0.01, chunk: 50 }
    }
}

fn sub(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn dot(a: Vector3, b: Vector3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// `8277A208`: the head's normal decides; a NaN dot passes, a NaN distance fails.
fn coplanar(r: &ContactRecord, s: &ContactRecord, rules: &ReductionRules) -> bool {
    let d = dot(r.normal, s.normal);
    if rules.normal_dot > d {
        return false;
    }
    let e0 = dot(r.normal, sub(r.p0, s.p0)).abs();
    let e1 = dot(r.normal, sub(r.p1, s.p1)).abs();
    let m = if e0 - e1 >= 0.0 { e1 } else { e0 };
    m < rules.plane_distance
}

/// `827798F8`: up to four of the group's points (indexes into `members`), ties keep the lowest index.
fn choose4(recs: &[ContactRecord], members: &[usize], d: Vector3) -> Vec<usize> {
    let p = |k: usize| recs[members[k]].p0;
    let q = |k: usize| recs[members[k]].p1;
    let n = members.len();
    let mut i0 = 0;
    let mut best = dot(d, sub(p(0), q(0)));
    for k in 1..n {
        let s = dot(d, sub(p(k), q(k)));
        if best > s {
            best = s;
            i0 = k;
        }
    }
    let a = p(i0);
    let (mut i1, mut best) = (0, dot(sub(p(0), a), sub(p(0), a)));
    for k in 1..n {
        let e = sub(p(k), a);
        if dot(e, e) > best {
            best = dot(e, e);
            i1 = k;
        }
    }
    if i0 == i1 {
        return vec![i0];
    }
    let e1 = sub(p(i1), a);
    let size = |k: usize| {
        let v = cross(sub(p(k), a), e1);
        dot(v, v)
    };
    let (mut i2, mut best) = (0, size(0));
    for k in 1..n {
        if size(k) > best {
            best = size(k);
            i2 = k;
        }
    }
    if i2 == i0 || i2 == i1 {
        return vec![i0, i1];
    }
    let e2 = sub(p(i2), a);
    let n1 = cross(d, e1);
    let n2 = cross(d, e2);
    let sgn = if dot(n1, e2) >= 0.0 { 1.0 } else { -1.0 };
    let u1 = Vector3::new(n1.x * sgn, n1.y * sgn, n1.z * sgn);
    let u2 = Vector3::new(n2.x * sgn, n2.y * sgn, n2.z * sgn);
    let (mut i3, mut best) = (usize::MAX, -1.0f32);
    for k in 0..n {
        let e = sub(p(k), a);
        let v = if dot(e, u1) > 0.0 {
            if dot(e, u2) > 0.0 { cross(e, e2) } else { cross(sub(e, e1), sub(e2, e1)) }
        } else {
            cross(e, e1)
        };
        if dot(v, v) > best {
            best = dot(v, v);
            i3 = k;
        }
    }
    let mut out = vec![i0, i1, i2];
    if i3 != usize::MAX && i3 != i0 && i3 != i1 && i3 != i2 {
        out.push(i3);
    }
    out
}

/// `82779EB0` over one chunk of one body pair's records: the kept indexes, ascending (records keep their order).
pub fn reduce_chunk(recs: &[ContactRecord], rules: &ReductionRules) -> Vec<usize> {
    let mut grouped = vec![false; recs.len()];
    let mut keep = Vec::with_capacity(recs.len());
    for i in 0..recs.len() {
        if grouped[i] {
            continue;
        }
        grouped[i] = true;
        let mut members = vec![i];
        for j in i + 1..recs.len() {
            if !grouped[j] && coplanar(&recs[i], &recs[j], rules) {
                grouped[j] = true;
                members.push(j);
            }
        }
        if members.len() > 4 {
            keep.extend(choose4(recs, &members, recs[i].normal).into_iter().map(|k| members[k]));
        } else {
            keep.extend(members);
        }
    }
    keep.sort_unstable();
    keep
}

/// [`reduce_chunk`] over consecutive chunks of `rules.chunk` records (indexes into `recs`, ascending); all records
/// when the reduction is off.
pub fn reduce(recs: &[ContactRecord], rules: &ReductionRules) -> Vec<usize> {
    if !rules.enabled {
        return (0..recs.len()).collect();
    }
    let chunk = rules.chunk.max(1);
    let mut out = Vec::with_capacity(recs.len());
    for start in (0..recs.len()).step_by(chunk) {
        let end = (start + chunk).min(recs.len());
        out.extend(reduce_chunk(&recs[start..end], rules).into_iter().map(|k| start + k));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(x: f32, z: f32, depth: f32) -> ContactRecord {
        ContactRecord { p0: Vector3::new(x, -depth, z), p1: Vector3::new(x, 0.0, z), normal: Vector3::new(0.0, 1.0, 0.0) }
    }

    /// A flat 3 x 3 grid of contacts on one plane keeps four: the deepest point, the farthest from it, the widest
    /// triangle and the point farthest outside it.
    #[test]
    fn a_coplanar_group_keeps_four_corner_points() {
        let mut recs = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                recs.push(rec(i as f32, j as f32, if (i, j) == (1, 1) { 0.02 } else { 0.01 }));
            }
        }
        let kept = reduce(&recs, &ReductionRules::default());
        assert_eq!(kept.len(), 4, "{kept:?}");
        assert!(kept.contains(&4), "the deepest point (the centre) heads the choice: {kept:?}");
        assert!(kept.windows(2).all(|w| w[0] < w[1]), "record order kept");
    }

    #[test]
    fn groups_of_four_or_fewer_and_other_planes_stay_whole() {
        let mut recs: Vec<_> = (0..4).map(|k| rec(k as f32, 0.0, 0.01)).collect();
        // A wall contact (normal +x) is its own group.
        recs.push(ContactRecord { p0: Vector3::new(5.0, 1.0, 0.0), p1: Vector3::new(5.1, 1.0, 0.0), normal: Vector3::new(1.0, 0.0, 0.0) });
        assert_eq!(reduce(&recs, &ReductionRules::default()), vec![0, 1, 2, 3, 4]);
        // Off: everything.
        let many: Vec<_> = (0..9).map(|k| rec(k as f32, (k % 3) as f32, 0.01)).collect();
        assert_eq!(reduce(&many, &ReductionRules { enabled: false, ..Default::default() }).len(), 9);
    }

    /// A full chunk is reduced on its own and the rest starts a new one (`82779E40`, b520): nothing is dropped.
    #[test]
    fn chunks_are_reduced_separately() {
        let recs: Vec<_> = (0..12).map(|k| rec((k % 4) as f32, (k / 4) as f32, 0.01)).collect();
        let rules = ReductionRules { chunk: 6, ..Default::default() };
        let kept = reduce(&recs, &rules);
        assert!(kept.iter().filter(|&&k| k < 6).count() <= 4 && kept.iter().filter(|&&k| k >= 6).count() <= 4, "{kept:?}");
        assert!(kept.iter().any(|&k| k >= 6));
    }

    #[test]
    fn the_result_is_deterministic() {
        let recs: Vec<_> = (0..20).map(|k| rec((k * 7 % 5) as f32 * 0.3, (k * 3 % 4) as f32 * 0.2, 0.005 + (k % 3) as f32 * 0.001)).collect();
        assert_eq!(reduce(&recs, &ReductionRules::default()), reduce(&recs, &ReductionRules::default()));
    }
}
