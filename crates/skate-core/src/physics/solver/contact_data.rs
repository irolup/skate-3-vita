//! Named view of the compiled contact and shared reaction records.
//! Fourth components retain the native packed layout, including inverse mass
//! alongside each body's first angular response.
use crate::madd::Madd as _;
type Components = [f32; 4];

pub(super) struct ContactRows {
    pub arm_a: Components,
    pub arm_b: Components,
    pub correction_columns: [Components; 3],
    pub static_friction: f32,
    pub dynamic_friction: f32,
    pub accumulated: Components,
    pub target: Components,
    axes: [Components; 3],
    angular_a: [Components; 3],
    angular_b: [Components; 3],
}
impl ContactRows {
    #[inline(always)]
    pub fn read(words: &[u32]) -> Self {
        assert_eq!(words.len(), 64);
        Self {
            arm_a: read(words, 0),
            arm_b: read(words, 1),
            correction_columns: core::array::from_fn(|i| read(words, 2 + i)),
            static_friction: f32::from_bits(words[15]),
            dynamic_friction: f32::from_bits(words[19]),
            accumulated: read(words, 5),
            target: read(words, 6),
            axes: core::array::from_fn(|i| read(words, 7 + 3 * i)),
            angular_a: core::array::from_fn(|i| read(words, 8 + 3 * i)),
            angular_b: core::array::from_fn(|i| read(words, 9 + 3 * i)),
        }
    }
    #[inline(always)]
    pub fn publish_impulses(&self, words: &mut [u32], impulse: Components) {
        write(words, 5, impulse);
    }
}

pub(super) struct Reaction {
    linear: Components,
    position: Components,
    angular: Components,
    orientation: Components,
}
impl Reaction {
    #[inline(always)]
    pub fn read(words: &[u32]) -> Self {
        assert_eq!(words.len(), 16);
        Self {
            linear: read(words, 0),
            position: read(words, 1),
            angular: read(words, 2),
            orientation: read(words, 3),
        }
    }
    #[inline(always)]
    pub fn point_position(&self, arm: Components) -> Components {
        point_correction(arm, self.position, self.orientation)
    }
    #[inline(always)]
    pub fn point_velocity(&self, arm: Components) -> Components {
        point_correction(arm, self.linear, self.angular)
    }
    #[inline(always)]
    pub fn apply_contact(&mut self, rows: &ContactRows, change: Components, body_a: bool) {
        let response = if body_a {
            &rows.angular_a
        } else {
            &rows.angular_b
        };
        let inverse_mass = response[0][3];
        for lane in 0..4 {
            let mut linear_impulse = rows.axes[0][lane].madd(change[0], 0.0);
            linear_impulse = rows.axes[1][lane].madd(change[1], linear_impulse);
            linear_impulse = rows.axes[2][lane].madd(change[2], linear_impulse);
            let position_impulse = rows.axes[0][lane].madd(change[3], 0.0);
            if body_a {
                for (axis, &impulse) in change[..3].iter().enumerate() {
                    self.angular[lane] = response[axis][lane].madd(impulse, self.angular[lane]);
                }
                self.orientation[lane] =
                    response[0][lane].madd(change[3], self.orientation[lane]);
                self.linear[lane] = linear_impulse.madd(inverse_mass, self.linear[lane]);
                self.position[lane] = position_impulse.madd(inverse_mass, self.position[lane]);
            } else {
                for (axis, &impulse) in change[..3].iter().enumerate() {
                    self.angular[lane] =
                        (-response[axis][lane]).madd(impulse, self.angular[lane]);
                }
                self.orientation[lane] =
                    (-response[0][lane]).madd(change[3], self.orientation[lane]);
                self.linear[lane] = (-linear_impulse).madd(inverse_mass, self.linear[lane]);
                self.position[lane] =
                    (-position_impulse).madd(inverse_mass, self.position[lane]);
            }
        }
    }
    #[inline(always)]
    pub fn write(&self, words: &mut [u32]) {
        for (row, value) in [self.linear, self.position, self.angular, self.orientation]
            .into_iter()
            .enumerate()
        {
            write(words, row, value);
        }
    }
}

// 82AE297C..2A18: linear + angular cross arm. Fuse the first product into
// linear before subtracting the second; a separate cross product rounds early.
#[inline(always)]
fn point_correction(arm: Components, linear: Components, angular: Components) -> Components {
    core::array::from_fn(|lane| {
        let next = if lane == 3 { 3 } else { (lane + 1) % 3 };
        let previous = if lane == 3 { 3 } else { (lane + 2) % 3 };
        let first = angular[next].madd(arm[previous], linear[lane]);
        (-angular[previous]).madd(arm[next], first)
    })
}
#[inline(always)]
fn read(words: &[u32], row: usize) -> Components {
    core::array::from_fn(|lane| f32::from_bits(words[4 * row + lane]))
}
#[inline(always)]
fn write(words: &mut [u32], row: usize, value: Components) {
    for (lane, value) in value.into_iter().enumerate() {
        words[4 * row + lane] = value.to_bits();
    }
}
