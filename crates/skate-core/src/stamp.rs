//! Where a tattoo lies on a body part: retail's placement maths (research of 8-9 Nov 2026, TU3; the notes are in
//! PROVENANCE.md). A placement is four numbers: the turn (radians), the scale and the offset across and up the part's
//! UV; the pages start from a table and the sticks move them a little a frame.
//!
//! - 82C2E3A8: the matrix `W` that takes a part's UV (a row vector `(u, v, 1)`) to the tattoo's picture:
//!   `T(tx, ty) T(-0.5, -0.5) R(turn) S(scale, 2 scale) T(0.5, 0.5)`, `R` with rows `(c, s)` and `(-s, c)`. The
//!   draw-time setter (82784B88) hands the shader its columns 0 and 1 (`i_customGraphic`, two rows of three).
//! - 82C2EBD8: a placement fits when the picture's four corners, taken back through `W` inverse, all lie in the
//!   part's allowed rectangle.
//! - 82C2E698: add the change to one number (the scale up to 15); when it no longer fits, nudge the offsets so the first
//!   corner out of the rectangle comes back in (by its overflow and 0.001), the across offset before the up one; and
//!   while it still does not fit, multiply the scale by 1.8751249.
//! - 82536C40: the rectangle and the picture's bounds come from the catalog ([`crate::cas_rules::Catalog::stamp_rect`]
//!   and `stamp_bounds`), the change from the sticks ([`RATES`]).

/// The scale never grows past this by a change (82C2E698).
const SCALE_CAP: f32 = 15.0;
/// A nudge goes this far past the edge, on its inside (`DAT_82063A48`).
const NUDGE: f32 = 0.001;
/// What a scale that does not fit is multiplied by again (`DAT_822F9210`).
const SHRINK: f32 = 1.875_124_9;

/// A cleared placement: no tattoo here (825315E8 and the reset of the region pages).
pub const NONE: Placement = Placement { turn: 0.0, scale: 1.0, across: -1.0, up: -1.0 };

/// What one unit of each input changes a frame (825FBAE8): the turn by the shoulder buttons, the scale by the left
/// stick's up and down, the offsets by the right stick (across and up). Inputs inside +-0.01 do nothing.
pub const RATES: [f32; 4] = [0.01, -0.1, -0.005, 0.005];
pub const DEAD_ZONE: f32 = 0.01;

/// A tattoo's placement on its part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub turn: f32,
    pub scale: f32,
    pub across: f32,
    pub up: f32,
}

impl Placement {
    pub fn from_values(v: [f32; 4]) -> Self {
        Self { turn: v[0], scale: v[1], across: v[2], up: v[3] }
    }

    pub fn values(self) -> [f32; 4] {
        [self.turn, self.scale, self.across, self.up]
    }

    /// `W` as rows of a row-vector matrix (82C2E3A8).
    pub fn matrix(self) -> [[f32; 3]; 3] {
        let (s, c) = self.turn.sin_cos();
        let (sx, sy) = (self.scale, 2.0 * self.scale);
        // Row vectors: the offset, the centring, the turn, the scale, then the centring back.
        let (ox, oy) = (self.across - 0.5, self.up - 0.5);
        // (u + ox) c - (v + oy) s, then (u + ox) s + (v + oy) c, scaled, then + 0.5.
        [
            [c * sx, s * sy, 0.0],
            [-s * sx, c * sy, 0.0],
            [(ox * c - oy * s) * sx + 0.5, (ox * s + oy * c) * sy + 0.5, 1.0],
        ]
    }

    /// The two rows of `i_customGraphic` (82784B88): the matrix's columns 0 and 1.
    pub fn custom_graphic(self) -> [[f32; 3]; 2] {
        let m = self.matrix();
        [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]]]
    }
}

fn invert(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let c = |r1: usize, c1: usize, r2: usize, c2: usize| m[r1][c1] * m[r2][c2] - m[r1][c2] * m[r2][c1];
    let det = m[0][0] * c(1, 1, 2, 2) - m[0][1] * c(1, 0, 2, 2) + m[0][2] * c(1, 0, 2, 1);
    let inv = 1.0 / det;
    [
        [c(1, 1, 2, 2) * inv, -c(0, 1, 2, 2) * inv, c(0, 1, 1, 2) * inv],
        [-c(1, 0, 2, 2) * inv, c(0, 0, 2, 2) * inv, -c(0, 0, 1, 2) * inv],
        [c(1, 0, 2, 1) * inv, -c(0, 0, 2, 1) * inv, c(0, 0, 1, 1) * inv],
    ]
}

/// The picture's four corners on the part, in the order 82C2E698 walks them (`bounds` are the picture's (top, bottom,
/// left, right): the corners are (left, bottom), (left, top), (right, bottom), (right, top)).
fn corners(p: Placement, bounds: [f32; 4]) -> [[f32; 2]; 4] {
    let inv = invert(p.matrix());
    let at = |x: f32, y: f32| [x * inv[0][0] + y * inv[1][0] + inv[2][0], x * inv[0][1] + y * inv[1][1] + inv[2][1]];
    let [top, bottom, left, right] = bounds;
    [at(left, top), at(left, bottom), at(right, top), at(right, bottom)]
}

/// 82C2EBD8: do the picture's corners all lie in `rect` = (top, bottom, left, right) on the part?
pub fn fits(p: Placement, rect: [f32; 4], bounds: [f32; 4]) -> bool {
    let [top, bottom, left, right] = rect;
    corners(p, bounds).iter().all(|&[x, y]| x >= left && x <= right && y >= bottom && y <= top)
}

/// 82C2E698: one number (0 turn, 1 scale, 2 across, 3 up) changes by `delta`, and the placement is brought back into
/// `rect` when it no longer fits.
pub fn step(p: Placement, value: usize, delta: f32, rect: [f32; 4], bounds: [f32; 4]) -> Placement {
    let mut v = p.values();
    if value < 4 {
        v[value] += delta;
        if value == 1 {
            v[1] = v[1].min(SCALE_CAP);
        }
    }
    let mut q = Placement::from_values(v);
    if fits(q, rect, bounds) {
        return q;
    }
    let [top, bottom, left, right] = rect;
    let c = corners(q, bounds);
    // Across: only when all corners lie on one side's outside of nothing (all below the right edge or all past the
    // left), the first corner out of [left, right] moves the offset by its overflow and a thousandth past.
    if c.iter().all(|k| k[0] <= right) || c.iter().all(|k| k[0] >= left) {
        if let Some(x) = c.iter().map(|k| k[0]).find(|&x| x < left || x > right) {
            let edge = if x < left { left } else { right };
            q.across += (if x - edge < 0.0 { -NUDGE } else { NUDGE }) + (x - edge);
        }
    }
    if c.iter().all(|k| k[1] >= bottom) || c.iter().all(|k| k[1] <= top) {
        if let Some(y) = c.iter().map(|k| k[1]).find(|&y| y < bottom || y > top) {
            let edge = if y < bottom { bottom } else { top };
            q.up += (if y - edge < 0.0 { -NUDGE } else { NUDGE }) + (y - edge);
        }
    }
    // Retail loops until it fits; the starts and the nudges above make that so for its rectangles. A bound of ours
    // (not retail's) keeps a rectangle the picture can never be moved into from spinning.
    for _ in 0..40 {
        if fits(q, rect, bounds) {
            break;
        }
        q.scale *= SHRINK;
    }
    q
}

/// The inputs' change a frame on each of the four numbers, past the dead zone (825FBAE8): `input` is the buttons'
/// turn, the left stick's up, and the right stick's across and up, each as the pad reports it.
pub fn change(value: usize, input: f32) -> Option<f32> {
    (input > DEAD_ZONE || input < -DEAD_ZONE).then(|| input * RATES[value])
}

/// Where a tattoo starts at a location 0 to 11 (0x830442B8, 32 bytes a location: the male placement, then the female).
pub fn start(location: u32, female: bool) -> Placement {
    const T: [[[f32; 4]; 2]; 12] = [
        [[0.0, 5.0, 0.25, 0.25], [0.0, 5.0, 0.25, 0.25]],
        [[0.0, 5.0, 0.25, 0.25], [0.0, 5.0, 0.25, 0.21]],
        [[1.571, 7.0, 0.25, 0.25], [1.571, 7.0, 0.25, 0.25]],
        [[0.0, 5.0, 0.25, 0.32], [0.0, 5.0, 0.25, 0.32]],
        [[0.0, 5.0, -0.25, 0.25], [0.0, 5.0, -0.25, 0.25]],
        [[0.0, 5.0, -0.25, 0.3], [0.0, 5.0, -0.25, 0.1]],
        [[0.0, 7.0, 0.25, -0.15], [0.0, 7.0, 0.25, -0.15]],
        [[0.0, 7.0, 0.234, -0.15], [0.0, 7.0, 0.25, -0.15]],
        [[0.0, 10.0, 0.17, -0.35], [0.0, 10.0, 0.19, -0.35]],
        [[0.0, 7.0, -0.234, -0.15], [0.0, 7.0, -0.25, -0.15]],
        [[0.0, 7.0, -0.25, -0.15], [0.0, 7.0, -0.25, -0.15]],
        [[0.0, 10.0, -0.17, -0.35], [0.0, 10.0, -0.19, -0.35]],
    ];
    Placement::from_values(T[(location as usize).min(11)][usize::from(female)])
}

/// The locations of each placement slot by region (0x83044438 shirt, 0x83044468 upper, 0x83044498 lower, 0x830444C8
/// board, 0x830444F8 hat): a region index to its location.
pub const LOCATIONS: [&[u32]; 5] = [&[0, 4, 6, 9], &[1, 5, 7, 10], &[8, 11], &[2], &[3]];

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    /// Takes a part's UV to the picture.
    fn apply(p: Placement, uv: [f32; 2]) -> [f32; 2] {
        let m = p.matrix();
        [uv[0] * m[0][0] + uv[1] * m[1][0] + m[2][0], uv[0] * m[0][1] + uv[1] * m[1][1] + m[2][1]]
    }

    #[test]
    fn the_matrix_is_the_offset_the_turn_the_scale_and_the_centre() {
        // No turn, scale 1, no offset: u stays, v goes through 2 v - 0.5.
        let p = Placement { turn: 0.0, scale: 1.0, across: 0.0, up: 0.0 };
        assert!(close(apply(p, [0.5, 0.5]), [0.5, 0.5]));
        assert!(close(apply(p, [0.25, 0.75]), [0.25, 1.0]));
        // The offset is added to the part's UV before everything.
        let p = Placement { turn: 0.0, scale: 2.0, across: 0.1, up: 0.0 };
        assert!(close(apply(p, [0.4, 0.5]), [0.5, 0.5]));
        // A quarter turn about the centre.
        let p = Placement { turn: std::f32::consts::FRAC_PI_2, scale: 1.0, across: 0.0, up: 0.0 };
        assert!(close(apply(p, [0.5, 0.5]), [0.5, 0.5]));
        assert!(close(apply(p, [0.75, 0.5]), [0.5, 1.0]));
    }

    #[test]
    fn the_shaders_rows_are_the_matrix_columns() {
        let p = Placement { turn: 0.3, scale: 4.0, across: 0.2, up: -0.1 };
        let [r0, r1] = p.custom_graphic();
        let uv = [0.3, 0.6];
        let out = apply(p, uv);
        assert!(close([r0[0] * uv[0] + r0[1] * uv[1] + r0[2], r1[0] * uv[0] + r1[1] * uv[1] + r1[2]], out));
    }

    #[test]
    fn the_inverse_takes_the_picture_back_to_the_part() {
        let p = Placement { turn: 0.4, scale: 3.0, across: 0.1, up: 0.2 };
        let c = corners(p, [1.0, 0.0, 0.0, 1.0]);
        for k in c {
            let back = apply(p, k);
            assert!(back[0] >= -1e-3 && back[0] <= 1.001 && back[1] >= -1e-3 && back[1] <= 1.001, "{back:?}");
        }
    }

    const RECT: [f32; 4] = [0.99, 0.45, 0.16, 0.38];
    const WHOLE: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

    #[test]
    fn a_start_fits_its_part_and_a_big_push_is_brought_back() {
        let arm = [0.99, 0.45, 0.16, 0.38];
        let p = step(start(6, false), 1, 0.0, arm, WHOLE);
        assert!(fits(p, arm, WHOLE), "{p:?}");
        // Pushing the tattoo far across never leaves the rectangle.
        let mut q = p;
        for _ in 0..200 {
            q = step(q, 2, -0.005 * 5.0, arm, WHOLE);
            assert!(fits(q, arm, WHOLE), "{q:?}");
        }
        for _ in 0..200 {
            q = step(q, 3, 0.005 * 5.0, arm, WHOLE);
            assert!(fits(q, arm, WHOLE), "{q:?}");
        }
    }

    #[test]
    fn the_scale_is_capped_and_shrunk_when_it_does_not_fit() {
        let mut p = Placement { turn: 0.0, scale: 14.9, across: 0.0, up: 0.0 };
        p = step(p, 1, 5.0, [100.0, -100.0, -100.0, 100.0], WHOLE);
        assert_eq!(p.scale, 15.0);
        // A scale that cannot fit grows its multiplier until it does.
        let tight = [0.8, 0.6, 0.2, 0.3];
        let start = start(7, false);
        let q = step(start, 1, 0.0, tight, WHOLE);
        assert!(fits(q, tight, WHOLE), "{q:?}");
        assert!(q.scale > start.scale);
    }

    #[test]
    fn small_inputs_do_nothing_and_the_rates_have_retails_signs() {
        assert_eq!(change(0, 0.005), None);
        assert_eq!(change(2, -0.5), Some(0.0025));
        assert_eq!(change(1, 1.0), Some(-0.1));
    }

    #[test]
    fn the_start_table_is_by_location_and_gender() {
        assert_eq!(start(1, true).up, 0.21);
        assert_eq!(start(2, false).turn, 1.571);
        assert_eq!(start(8, true).across, 0.19);
        assert_eq!(start(11, false).up, -0.35);
    }
}
