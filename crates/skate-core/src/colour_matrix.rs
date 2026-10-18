//! Retail post FX colour matrices (`post_fx` vault records `colour_matrix`, `colour_matrix_zombiemode`, ...; research
//! b100). Each record has a near and a far band; a band's values build one 3x4 affine colour matrix with
//! `sub_827F13D8` [code]: contrast about a midpoint plus an add, then saturation with luma weights (0.3, 0.6, 0.1),
//! then a per-channel multiply: `out = multiply * Sat_s(contrast * (x - midpoint) + midpoint + add)`.
//!
//! The multiply-last order is [inferred] from the instruction order (b100); the base record is the identity.

/// One band's record values (`near_*` / `far_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourBand {
    pub multiply: [f32; 3],
    pub add: [f32; 3],
    pub contrast: f32,
    pub contrast_midpoint: f32,
    pub saturation: f32,
}

impl Default for ColourBand {
    /// The base `colour_matrix` record: identity.
    fn default() -> Self {
        Self { multiply: [1.0; 3], add: [0.0; 3], contrast: 1.0, contrast_midpoint: 0.5, saturation: 1.0 }
    }
}

/// A record: near and far bands and their distances (`distance_near` / `distance_far`, base 50 / 100 m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourMatrixRecord {
    pub near: ColourBand,
    pub far: ColourBand,
    pub distance_near: f32,
    pub distance_far: f32,
}

impl Default for ColourMatrixRecord {
    fn default() -> Self {
        Self { near: ColourBand::default(), far: ColourBand::default(), distance_near: 50.0, distance_far: 100.0 }
    }
}

/// Luma weights of the saturation step (`0x820D06C0` 0.3, `0x821EE79C` 0.6, `0x820641A8` 0.1) [code].
pub const LUMA: [f32; 3] = [0.3, 0.6, 0.1];

/// The band's 3x4 affine matrix: row `i` gives output channel `i` as `row[0..3] . rgb + row[3]` (`sub_827F13D8`).
pub fn build(b: &ColourBand) -> [[f32; 4]; 3] {
    let s = b.saturation;
    // S: s * I + (1 - s) * (1, 1, 1)^T LUMA.
    let sat = |i: usize, j: usize| if i == j { s } else { 0.0 } + (1.0 - s) * LUMA[j];
    // C: contrast on the diagonal, offset midpoint - contrast * midpoint + add.
    let offset: [f32; 3] = std::array::from_fn(|j| b.contrast_midpoint - b.contrast * b.contrast_midpoint + b.add[j]);
    std::array::from_fn(|i| {
        let mut row = [0.0; 4];
        for j in 0..3 {
            row[j] = b.multiply[i] * sat(i, j) * b.contrast;
            row[3] += b.multiply[i] * sat(i, j) * offset[j];
        }
        row
    })
}

/// Apply a matrix to a colour.
pub fn apply(m: &[[f32; 4]; 3], rgb: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| m[i][0] * rgb[0] + m[i][1] * rgb[1] + m[i][2] * rgb[2] + m[i][3])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    /// The base record is the identity.
    #[test]
    fn the_base_record_is_the_identity() {
        let m = build(&ColourBand::default());
        for c in [[0.0, 0.0, 0.0], [1.0, 0.5, 0.25], [0.2, 0.9, 0.4]] {
            assert!(close(apply(&m, c), c), "{c:?}");
        }
    }

    /// The zombie near band (`colour_matrix_zombiemode`: multiply (2.0, 1.4, 1.0), saturation 0.5, contrast 1.3,
    /// midpoint 0.5, add 0): grey stays grey before the multiply, so mid grey comes out yellow (1.0, 0.7, 0.5).
    #[test]
    fn the_zombie_band_turns_grey_yellow() {
        let near = ColourBand { multiply: [2.0, 1.4, 1.0], add: [0.0; 3], contrast: 1.3, contrast_midpoint: 0.5, saturation: 0.5 };
        let m = build(&near);
        assert!(close(apply(&m, [0.5; 3]), [1.0, 0.7, 0.5]));
        // Saturation 0.5 halves the distance to luma: pure red (contrast 1, multiply 1) -> (0.65, 0.15, 0.15).
        let sat = build(&ColourBand { saturation: 0.5, ..Default::default() });
        assert!(close(apply(&sat, [1.0, 0.0, 0.0]), [0.65, 0.15, 0.15]));
        // The far band's add -0.4 darkens: mid grey with contrast 1.5 -> 0.1 before the multiply.
        let far = ColourBand { multiply: [1.0; 3], add: [-0.4; 3], contrast: 1.5, contrast_midpoint: 0.5, saturation: 1.0 };
        assert!(close(apply(&build(&far), [0.5; 3]), [0.1; 3]));
    }
}
