use super::arithmetic::{dot, scale, vector};
use super::separating_axis_candidates;
use super::axes::{edge_candidates, normal_candidates};

/// Verified GP dispatch IDs from mapped TU3 table 82FD5870. Cylinder (5) and
/// invalid (0) are outside the board primitive set and have no fallback here.
#[derive(Clone, Copy, Debug)]
pub enum PrimitiveKind {
    Sphere,
    Capsule,
    Triangle,
    Box,
}

/// Full single-direction GP interval callbacks: sphere82ADD800,
/// capsule82AD99C8, triangle82ADE3B8, box82AD8508. Output is the native48-byte
/// interval; the last16bytes are untouched. Radius/padding is handled by caller.
pub fn project_direction(
    gp: &[u32; 48],
    kind: PrimitiveKind,
    direction: [u32; 4],
    output: &mut [u32; 12],
) {
    project(gp, kind, direction, output, false);
}

/// Full batch GP callbacks: sphere82ADD820, capsule82AD9A88,
/// triangle82ADE400, box82AD86B0. Box batch scales the axes *before* dot products;
/// its single callback scales the dot products instead. Preserve that difference.
pub fn project_directions(
    gp: &[u32; 48],
    kind: PrimitiveKind,
    directions: &[[u32; 4]],
    output: &mut [[u32; 12]],
) {
    assert!(output.len() >= directions.len());
    for (&direction, interval) in directions.iter().zip(output) {
        project(gp, kind, direction, interval, true);
    }
}

fn project(
    gp: &[u32; 48],
    kind: PrimitiveKind,
    direction: [u32; 4],
    output: &mut [u32; 12],
    batch: bool,
) {
    let direction = vector(&direction, 0);
    let center = dot(vector(gp, 0), direction);
    let (min, max) = match kind {
        PrimitiveKind::Sphere => (center, center),
        PrimitiveKind::Capsule => {
            let radius = dot(vector(gp, 16), direction).abs() * f32::from_bits(gp[28]);
            (center - radius, center + radius)
        }
        PrimitiveKind::Triangle => {
            let second = dot(vector(gp, 8), direction);
            let third = dot(vector(gp, 12), direction);
            // VMX min/max select the second operand on unordered or equal.
            let min = if center < second { center } else { second };
            let max = if center > second { center } else { second };
            (
                if min < third { min } else { third },
                if max > third { max } else { third },
            )
        }
        PrimitiveKind::Box => {
            let extent: [f32; 3] = std::array::from_fn(|i| {
                let axis = vector(gp, 4 + i * 4);
                let half = f32::from_bits(gp[28 + i]);
                if batch {
                    dot(scale(axis, half), direction).abs()
                } else {
                    (dot(axis, direction) * half).abs()
                }
            });
            let radius = (extent[0] + extent[1]) + extent[2];
            (center - radius, center + radius)
        }
    };
    output[..4].fill(min.to_bits());
    output[4..8].fill(max.to_bits());
}

/// Complete generic82ACF070 plus recovered GP batch callbacks for board shapes.
/// Specialized triangle/box82ACF950 and box/box82ACF448 dispatch entries must
/// still be selected by the caller; this function is only the generic entry.
/// Returns (broadcast separation, oriented four-lane axis).
pub fn best_separating_direction(
    a: &[u32; 48],
    a_kind: PrimitiveKind,
    b: &[u32; 48],
    b_kind: PrimitiveKind,
) -> ([u32; 4], [u32; 4]) {
    let mut directions = [[0; 4]; 16];
    let mut count = separating_axis_candidates(a, b, &mut directions);
    if count == 0 {
        directions[0] = [0, 0, 1.0f32.to_bits(), 0];
        count = 1;
    }
    // skate_vita: each candidate is projected just before it is compared, into
    // one interval per shape, instead of two zero-filled 16-slot arrays (1.5 KB of
    // zeroing per call on the Vita). `project` is pure, so every interval and the
    // selection order are unchanged; these are the batch callbacks, as before.
    let mut selected = 0;
    let mut best = 0.0;
    let mut flip = false;
    for index in 0..count {
        let mut a_interval = [0; 12];
        let mut b_interval = [0; 12];
        project(a, a_kind, directions[index], &mut a_interval, true);
        project(b, b_kind, directions[index], &mut b_interval, true);
        let forward = f32::from_bits(a_interval[0]) - f32::from_bits(b_interval[4]);
        let reverse = f32::from_bits(b_interval[0]) - f32::from_bits(a_interval[4]);
        // fcmpu followed by ble (!gt): unordered selects reverse without flip.
        let (separation, reversed) = if forward > reverse {
            (forward, true)
        } else {
            (reverse, false)
        };
        if index == 0 || separation > best {
            best = separation;
            selected = index;
            flip = reversed;
        }
    }
    let mut normal = directions[selected];
    if flip {
        normal.iter_mut().for_each(|v| *v ^= 0x8000_0000);
    }
    ([best.to_bits(); 4], normal)
}

/// skate_vita: `best_separating_direction` for callers that discard its
/// result whenever the separation is greater than `reject_above` (the
/// `separation > fatness + limit + fatness` gate). Candidates are produced and
/// compared in the same order; the face normals come first and cost no
/// normalization. Returns None as soon as the running best is greater than
/// `reject_above`: later candidates only replace it with a greater separation,
/// so the full search would be rejected too. (A NaN best is never replaced and
/// is never greater, so it runs to the end, as before.) Otherwise the result
/// equals `best_separating_direction`.
pub fn best_separating_direction_unless_above(
    a: &[u32; 48],
    a_kind: PrimitiveKind,
    b: &[u32; 48],
    b_kind: PrimitiveKind,
    reject_above: f32,
) -> Option<([u32; 4], [u32; 4])> {
    let mut directions = [[0; 4]; 16];
    let mut selected = 0;
    let mut best = 0.0;
    let mut flip = false;
    let mut consider = |index: usize, direction: [u32; 4]| {
        let mut a_interval = [0; 12];
        let mut b_interval = [0; 12];
        project(a, a_kind, direction, &mut a_interval, true);
        project(b, b_kind, direction, &mut b_interval, true);
        let forward = f32::from_bits(a_interval[0]) - f32::from_bits(b_interval[4]);
        let reverse = f32::from_bits(b_interval[0]) - f32::from_bits(a_interval[4]);
        // fcmpu followed by ble (!gt): unordered selects reverse without flip.
        let (separation, reversed) = if forward > reverse {
            (forward, true)
        } else {
            (reverse, false)
        };
        if index == 0 || separation > best {
            best = separation;
            selected = index;
            flip = reversed;
        }
        best > reject_above
    };
    let normals = normal_candidates(a, b, &mut directions);
    for index in 0..normals {
        if consider(index, directions[index]) {
            return None;
        }
    }
    let mut count = edge_candidates(a, b, &mut directions, normals);
    if count == 0 {
        directions[0] = [0, 0, 1.0f32.to_bits(), 0];
        count = 1;
    }
    for index in normals..count {
        if consider(index, directions[index]) {
            return None;
        }
    }
    let mut normal = directions[selected];
    if flip {
        normal.iter_mut().for_each(|v| *v ^= 0x8000_0000);
    }
    Some(([best.to_bits(); 4], normal))
}

#[cfg(test)]
mod early_reject_tests {
    use super::*;
    use crate::{
        math::{Basis3, Vector3},
        physics::{collision::Sphere, world_contact::{ContactPrimitive, triangle_from_volume}},
    };
    use super::super::primitive_query::geometry::pack;

    /// The early-stopping search agrees with the full one for every pair and
    /// threshold: None exactly when the full separation is above the threshold.
    #[test]
    fn unless_above_matches_full_search() {
        let mut state = 0x2545_f491_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 4001) as f32 / 1000. - 2.
        };
        let unit = |v: [f32; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-3);
            Vector3::new(v[0] / l, v[1] / l, v[2] / l)
        };
        let shape = |kind: u32, next: &mut dyn FnMut() -> f32| -> ContactPrimitive {
            let c = Vector3::new(next(), next(), next());
            match kind % 4 {
                0 => ContactPrimitive::Sphere(Sphere { center: c, radius: next().abs() * 0.3 }),
                1 => ContactPrimitive::Capsule {
                    center: c,
                    axis: unit([next(), next(), next()]),
                    half_length: next().abs() * 0.5,
                    radius: next().abs() * 0.2,
                },
                2 => ContactPrimitive::Triangle(triangle_from_volume(
                    [c, Vector3::new(c.x + next(), c.y + next(), c.z), Vector3::new(c.x, c.y + next(), c.z + next())],
                    next().abs() * 0.05,
                    [0.5; 3],
                    0,
                )),
                _ => {
                    let (s, k) = (next() * 0.5, (1. - (next() * 0.5).powi(2)).max(0.).sqrt());
                    ContactPrimitive::RoundedBox {
                        center: c,
                        basis: Basis3 { columns: [[k, s, 0.], [-s, k, 0.], [0., 0., 1.]] },
                        half_extents: Vector3::new(next().abs(), next().abs(), next().abs()),
                        radius: next().abs() * 0.1,
                    }
                }
            }
        };
        let (mut rejected, mut kept) = (0, 0);
        for i in 0..40_000u32 {
            let (a, a_kind) = pack(shape(i, &mut next));
            let (b, b_kind) = pack(shape(i / 4 + 1, &mut next));
            let full = best_separating_direction(&a, a_kind, &b, b_kind);
            let best = f32::from_bits(full.0[0]);
            for threshold in [next() * 2., best, -best, f32::INFINITY, f32::NAN] {
                let early = best_separating_direction_unless_above(&a, a_kind, &b, b_kind, threshold);
                if best > threshold {
                    assert!(early.is_none(), "pair {i}: {best} > {threshold} must reject");
                    rejected += 1;
                } else {
                    assert_eq!(early, Some(full), "pair {i}, threshold {threshold}");
                    kept += 1;
                }
            }
        }
        assert!(rejected > 10_000 && kept > 10_000, "{rejected} / {kept}");
    }
}
