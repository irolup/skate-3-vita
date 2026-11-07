//! Camera triangle leaf82771D08 stores the authored face normal, clamps every
//! fraction and retains the first equal-distance hit in world traversal order.
use skate_core::{camera::FatLineResult, math::Vector3,
    physics::{board_world::BoardWorld, triangle_query::{TriangleLineHit, triangle_segment}}};

pub(crate) fn line(world: &BoardWorld, start: [f32; 4], end: [f32; 4], radius: f32)
    -> Result<FatLineResult, String> {
    if !radius.is_finite() || radius < 0.0 {
        return Err("Invalid stock camera collision radius".into());
    }
    let start = vector(start);
    let delta = Vector3::new(end[0] - start.x, end[1] - start.y, end[2] - start.z);
    let mut fraction = f32::MAX;
    let mut result = no_hit();
    for (_, entry) in world.line_candidates(start, vector(end), radius) {
        let mut hit = TriangleLineHit { position: Vector3::ZERO, normal: Vector3::ZERO,
            fraction: 0.0, volume_parameter: [0.0; 3] };
        // Zero triangle fatness and zero material exclusion mask are native.
        // Subject actor exclusion is preserved by BoardWorld's world-only data.
        if triangle_segment(&mut hit, start, delta, entry.triangle.vertices, radius, 0.0) {
            let lower = if -hit.fraction >= 0.0 { 0.0 } else { hit.fraction };
            let candidate = if 1.0 - lower >= 0.0 { lower } else { 1.0 };
            if candidate < fraction {
                fraction = candidate;
                result = FatLineResult { position: lanes(hit.position),
                    normal: lanes(entry.triangle.feature.normal), fraction: candidate,
                    hit: 1, surface: entry.tag };
            }
        }
    }
    if let Some(external) = world.external_line(start, vector(end), radius) {
        let hit = external.hit.geometry;
        let candidate = hit.fraction.clamp(0.0, 1.0);
        if candidate < fraction {
            result = FatLineResult { position: lanes(hit.position), normal: lanes(hit.normal),
                fraction: candidate, hit: 1, surface: external.hit.tag };
        }
    }
    Ok(result)
}
/// skate_vita: the nearest surface a fat line meets from its front among the world's first `rows` triangles: its fraction
/// of the line and its normal (the Object Dropper's camera against a park's own map, which ends where the layout's rows
/// start). A surface met from behind or skimmed along does not stop it, so a camera behind one can still come out.
pub(crate) fn line_in_rows(world: &BoardWorld, start: [f32; 3], end: [f32; 3], rows: usize, radius: f32) -> Option<(f32, [f32; 3])> {
    let (from, to) = (Vector3::new(start[0], start[1], start[2]), Vector3::new(end[0], end[1], end[2]));
    let delta = Vector3::new(to.x - from.x, to.y - from.y, to.z - from.z);
    let length = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
    let mut nearest = None;
    for (row, entry) in world.line_candidates(from, to, radius) {
        if row >= rows {
            continue;
        }
        // Only a surface the line goes into: not one it skims along (a wall leaning a hair, the eye moving down against it).
        let n = entry.triangle.feature.normal;
        if n.x * delta.x + n.y * delta.y + n.z * delta.z >= -0.02 * length {
            continue;
        }
        let mut hit = TriangleLineHit { position: Vector3::ZERO, normal: Vector3::ZERO, fraction: 0.0, volume_parameter: [0.0; 3] };
        if triangle_segment(&mut hit, from, delta, entry.triangle.vertices, radius, 0.0) {
            let fraction = hit.fraction.clamp(0.0, 1.0);
            if nearest.is_none_or(|(f, _)| fraction < f) {
                nearest = Some((fraction, [n.x, n.y, n.z]));
            }
        }
    }
    nearest
}
pub(super) fn no_hit() -> FatLineResult {
    FatLineResult { position: [0.0; 4], normal: [0.0; 4], fraction: 0.0, hit: 0, surface: 0 }
}
fn vector(v: [f32; 4]) -> Vector3 { Vector3::new(v[0], v[1], v[2]) }
fn lanes(v: Vector3) -> [f32; 4] { [v.x, v.y, v.z, 0.0] }
