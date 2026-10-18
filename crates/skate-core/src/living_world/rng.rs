//! Explicit seeded RNG for population decisions (retail uses the CRT `rand()` and a world RNG,
//! `sub_82970628`; we keep the same draw shapes, e.g. `rand() % 400`, on our own generator so a
//! session replays exactly from its seed). No global state.

/// SplitMix64 step: a good 64-bit mix, used for seeding and for deriving sub-seeds.
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Derive a stable sub-seed from a parent seed and a sequence of labels.
pub fn derive(seed: u64, labels: &[u64]) -> u64 {
    labels.iter().fold(mix64(seed), |acc, &l| mix64(acc ^ mix64(l)))
}

/// PCG32 (XSH RR). Deterministic, small, portable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut rng = Self { state: 0, inc: (mix64(seed ^ 0xDA3E_39CB_94B9_5BDB) << 1) | 1 };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(mix64(seed));
        rng.next_u32();
        rng
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// `rand() % n` (retail form; n > 0).
    pub fn modulo(&mut self, n: u32) -> u32 {
        self.next_u32() % n.max(1)
    }

    /// A u32 scaled by 2^-32: [0, 1). Retail scales its world RNG the same way
    /// (`0x822F88F4` = 2^-32 in `sub_82E17508`).
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() as f64 * (1.0 / 4_294_967_296.0)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream_and_different_seeds_differ() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let mut c = Rng::new(8);
        let sa: Vec<u32> = (0..16).map(|_| a.next_u32()).collect();
        let sb: Vec<u32> = (0..16).map(|_| b.next_u32()).collect();
        let sc: Vec<u32> = (0..16).map(|_| c.next_u32()).collect();
        assert_eq!(sa, sb);
        assert_ne!(sa, sc);
    }

    #[test]
    fn unit_is_in_range() {
        let mut r = Rng::new(1);
        for _ in 0..10_000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
        assert_eq!(derive(1, &[2, 3]), derive(1, &[2, 3]));
        assert_ne!(derive(1, &[2, 3]), derive(1, &[3, 2]));
    }
}
