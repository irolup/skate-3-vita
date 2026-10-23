//! Teleport arrival ground fit-up, retail `82598428` (research b475): one straight ray down through the arrival point,
//! and on a hit only the translation becomes hit + `rest` up; rotation is never touched, a miss keeps the matrix.

/// The fit-up's values (defaults = retail's constants `DAT_821814A0`, `DAT_820BD5C4`, `DAT_820994B4`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundFit {
    pub enabled: bool,
    /// Ray start above the arrival point, m.
    pub up: f32,
    /// Ray length, m.
    pub length: f32,
    /// Arrival height above the hit, m.
    pub rest: f32,
    /// The ray also hits dynamic props and traffic cars (retail query mask 7 bits 2 and 4, research b493).
    pub props: bool,
    pub vehicles: bool,
}

impl Default for GroundFit {
    fn default() -> Self {
        Self { enabled: true, up: 0.75, length: 500.0, rest: 0.15, props: true, vehicles: true }
    }
}

/// `cast(start, end)` returns the first hit position along the segment (world y up).
pub fn fit_arrival_to_ground(
    matrix: [[f32; 4]; 4],
    fit: GroundFit,
    cast: impl FnOnce([f32; 3], [f32; 3]) -> Option<[f32; 3]>,
) -> [[f32; 4]; 4] {
    if !fit.enabled {
        return matrix;
    }
    let [x, y, z, _] = matrix[3];
    let start = [x, y + fit.up, z];
    let end = [x, start[1] - fit.length, z];
    let mut out = matrix;
    if let Some(hit) = cast(start, end) {
        out[3][0] = hit[0];
        out[3][1] = hit[1] + fit.rest;
        out[3][2] = hit[2];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: f32) -> [[f32; 4]; 4] {
        [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [3., y, -4., 1.]]
    }
    fn floor(height: f32) -> impl FnOnce([f32; 3], [f32; 3]) -> Option<[f32; 3]> {
        move |s, e| (s[1] >= height && e[1] <= height).then_some([s[0], height, s[2]])
    }

    #[test]
    fn f1_a_hit_below_sets_only_the_height() {
        let out = fit_arrival_to_ground(at(2.0), GroundFit::default(), floor(0.0));
        assert_eq!(out[3], [3., 0.15, -4., 1.]);
        assert_eq!(out[..3], at(2.0)[..3]);
    }

    #[test]
    fn f2_the_ray_starts_above_and_runs_straight_down() {
        fit_arrival_to_ground(at(2.0), GroundFit::default(), |s, e| {
            assert_eq!(s, [3., 2.75, -4.]);
            assert_eq!(e, [3., 2.75 - 500., -4.]);
            None
        });
    }

    #[test]
    fn f3_just_under_a_surface_comes_up_onto_it() {
        assert_eq!(fit_arrival_to_ground(at(-0.5), GroundFit::default(), floor(0.0))[3][1], 0.15);
    }

    #[test]
    fn f4_a_miss_keeps_the_matrix() {
        let m = at(7.0);
        assert_eq!(fit_arrival_to_ground(m, GroundFit::default(), |_, _| None), m);
    }

    #[test]
    fn f5_a_tilted_arrival_keeps_its_rotation() {
        let (s, c) = 30f32.to_radians().sin_cos();
        let mut m = at(2.0);
        m[0] = [c, s, 0., 0.];
        m[1] = [-s, c, 0., 0.];
        let out = fit_arrival_to_ground(m, GroundFit::default(), floor(0.0));
        assert_eq!(out[..3], m[..3]);
    }

    #[test]
    fn f6_disabled_never_casts() {
        let fit = GroundFit { enabled: false, ..GroundFit::default() };
        let m = at(2.0);
        assert_eq!(fit_arrival_to_ground(m, fit, |_, _| panic!("cast while disabled")), m);
    }

    #[test]
    fn f7_the_rest_height_is_data() {
        let fit = GroundFit { rest: 0.5, ..GroundFit::default() };
        assert_eq!(fit_arrival_to_ground(at(2.0), fit, floor(1.0))[3][1], 1.5);
    }
}
