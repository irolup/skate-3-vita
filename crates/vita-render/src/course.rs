//! The procedural Course's render mesh (baked lighting only), built from
//! the same quads as its collision (`skate_sim::live::course_surfaces`): flat per-face
//! N.L lighting in the vertex colours, skate-game's group colours, and a 2 m checker on
//! the floor so speed reads on screen.
use crate::math::{AMBIENT, DIFFUSE, light_direction};
use glam::Vec3;

/// 16 bytes: position (3 × f32), colour (4 × u8, normalized).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldVertex {
    pub position: [f32; 3],
    pub color: [u8; 4],
}
pub const WORLD_STRIDE: usize = 16;

/// skate-game's test-world colours (sRGB): floor, platform/ramp/rails, half pipe,
/// climbing block.
pub const GROUP_COLORS: [[f32; 3]; 4] = [
    [0.16, 0.19, 0.21],
    [0.48, 0.35, 0.22],
    [0.30, 0.43, 0.48],
    [0.24, 0.48, 0.31],
];
/// Floor checker cell (m) and its brightness step.
const CELL: f32 = 2.0;
const CHECKER: f32 = 0.07;

/// Triangle list (counter-clockwise front faces, like the collision's (0, 2, 1) and
/// (0, 3, 2) split).
pub fn mesh(groups: &[Vec<[[f32; 3]; 4]>; 4]) -> Vec<WorldVertex> {
    let mut out = Vec::new();
    for (group, quads) in groups.iter().enumerate() {
        for quad in quads {
            let q = quad.map(Vec3::from_array);
            if group == 0 {
                floor(&mut out, q);
            } else {
                face(&mut out, q, GROUP_COLORS[group], 1.0);
            }
        }
    }
    out
}

fn normal(q: [Vec3; 4]) -> Vec3 {
    (q[2] - q[0]).cross(q[1] - q[0]).normalize_or_zero()
}

fn shade(base: [f32; 3], n: Vec3, scale: f32) -> [u8; 4] {
    let lit = (AMBIENT + DIFFUSE * n.dot(light_direction()).max(0.0)) * scale;
    let c = base.map(|v| ((v * lit).clamp(0.0, 1.0) * 255.0).round() as u8);
    [c[0], c[1], c[2], 255]
}

fn face(out: &mut Vec<WorldVertex>, q: [Vec3; 4], base: [f32; 3], scale: f32) {
    let color = shade(base, normal(q), scale);
    for i in [0, 2, 1, 0, 3, 2] {
        out.push(WorldVertex {
            position: q[i].to_array(),
            color,
        });
    }
}

/// The floor quad split into `CELL`-sized cells with alternating brightness.
fn floor(out: &mut Vec<WorldVertex>, q: [Vec3; 4]) {
    // Bilinear over the quad: q0 → q1 is one edge, q0 → q3 the other.
    let (u_len, v_len) = ((q[1] - q[0]).length(), (q[3] - q[0]).length());
    let (nu, nv) = ((u_len / CELL).round().max(1.0) as usize, (v_len / CELL).round().max(1.0) as usize);
    let at = |i: usize, j: usize| {
        let (u, v) = (i as f32 / nu as f32, j as f32 / nv as f32);
        let a = q[0].lerp(q[1], u);
        let b = q[3].lerp(q[2], u);
        a.lerp(b, v)
    };
    for j in 0..nv {
        for i in 0..nu {
            let cell = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            let scale = if (i + j) % 2 == 0 { 1.0 } else { 1.0 - CHECKER };
            face(out, cell, GROUP_COLORS[0], scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(y: f32, size: f32) -> [[f32; 3]; 4] {
        // The collision floor's corner order.
        [[-size, y, -size], [size, y, -size], [size, y, size], [-size, y, size]]
    }

    #[test]
    fn floor_is_subdivided_and_faces_up() {
        let groups = [vec![square(-1.5, 50.0)], vec![], vec![], vec![]];
        let m = mesh(&groups);
        assert_eq!(m.len(), 50 * 50 * 6);
        let q = square(-1.5, 50.0).map(Vec3::from_array);
        assert!(normal(q).y > 0.99, "{:?}", normal(q));
        // Two alternating brightnesses.
        let mut colors: Vec<_> = m.iter().map(|v| v.color).collect();
        colors.dedup();
        assert!(colors.len() > 2);
    }

    #[test]
    fn other_groups_are_two_triangles_per_quad() {
        let groups = [vec![], vec![square(0.0, 1.0)], vec![square(1.0, 1.0); 2], vec![]];
        assert_eq!(mesh(&groups).len(), 3 * 6);
    }
}
