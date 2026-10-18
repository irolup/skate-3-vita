//! Multiply-add switch (skate_vita addition).
//!
//! The Xbox 360 had fused `fmadds`, so the reference uses `f32::mul_add` for bit
//! parity. The Vita's Cortex-A9 (VFPv3/NEON) has no fused multiply-add, so each
//! `mul_add` becomes a slow software `fmaf` call. Every `.mul_add(` in skate-core and
//! skate-sim was replaced mechanically by `.madd(`:
//! - by default `madd` *is* `mul_add` (bit-identical to the reference);
//! - with the `no-fma` feature it is a separate multiply and add, which the Vita build
//!   uses. The parity oracle measures the drift.
//!
//! The switch is an opt-out `no-fma` rather than an opt-in `exact-fma`,
//! because Cargo unifies features: skate-data and skate-sim depend on
//! skate-core with default features, so a default-on `exact-fma` could never be
//! turned off for the Vita.
/// f32 `madd` calls in soft-fma builds (`madd-count` diagnostic builds only: the atomic
/// increment itself costs a lot on the Vita, so timing builds must not include it).
#[cfg(feature = "madd-count")]
pub static MADD_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub trait Madd {
    fn madd(self, a: Self, b: Self) -> Self;
}

#[cfg(not(feature = "madd-audit"))]
impl Madd for f32 {
    #[inline(always)]
    fn madd(self, a: f32, b: f32) -> f32 {
        #[cfg(not(any(feature = "no-fma", feature = "fma-f64", feature = "soft-fma")))]
        {
            self.mul_add(a, b)
        }
        #[cfg(all(feature = "soft-fma", not(feature = "no-fma")))]
        {
            #[cfg(feature = "madd-count")]
            MADD_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            fmaf_exact(self, a, b)
        }
        #[cfg(feature = "no-fma")]
        {
            self * a + b
        }
        // Diagnostic only: what newlib's software fmaf computes on the Vita (exact
        // product in double, one double add, then round to float: double rounding).
        #[cfg(all(feature = "fma-f64", not(any(feature = "no-fma", feature = "soft-fma"))))]
        {
            (f64::from(self) * f64::from(a) + f64::from(b)) as f32
        }
    }
}

/// Diagnostic build only (`madd-audit`, never shipped): unfused everywhere except in
/// the source files named in `MADD_FUSED_FILES` (comma-separated path substrings).
/// Used to bisect which routine needs fused arithmetic under `no-fma`.
#[cfg(feature = "madd-audit")]
impl Madd for f32 {
    #[track_caller]
    fn madd(self, a: f32, b: f32) -> f32 {
        if audit::fused_here(std::panic::Location::caller().file()) {
            self.mul_add(a, b)
        } else {
            self * a + b
        }
    }
}

#[cfg(feature = "madd-audit")]
mod audit {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static FILES: OnceLock<Vec<String>> = OnceLock::new();
    thread_local! {
        /// Decision per call-site file, keyed by the address of its `'static` name.
        static CACHE: RefCell<HashMap<usize, bool>> = RefCell::new(HashMap::new());
    }
    pub fn fused_here(file: &'static str) -> bool {
        CACHE.with(|cache| {
            *cache.borrow_mut().entry(file.as_ptr() as usize).or_insert_with(|| {
                FILES
                    .get_or_init(|| {
                        std::env::var("MADD_FUSED_FILES")
                            .unwrap_or_default()
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .iter()
                    .any(|f| file.contains(f.as_str()))
            })
        })
    }
}

impl Madd for f64 {
    #[inline(always)]
    fn madd(self, a: f64, b: f64) -> f64 {
        #[cfg(not(feature = "no-fma"))]
        {
            self.mul_add(a, b)
        }
        #[cfg(feature = "no-fma")]
        {
            self * a + b
        }
    }
}

/// Correctly rounded `fma(x, y, z)` for f32 using only IEEE double multiply/add (no
/// hardware FMA), bit-identical to `f32::mul_add` for every input. The Vita build uses
/// it through the `soft-fma` feature: newlib's `fmaf` on the Vita rounds twice (double,
/// then float) and differs from a real FMA.
///
/// - The product of two f32 is exact in f64 (24 + 24 bits, exponents in range).
/// - TwoSum gives the exact rounding error `e` of `s = xy + z` in f64.
/// - Round-to-odd: if `e != 0` and `s` has an even last bit, move `s` one ulp toward
///   the exact value. Rounding a round-to-odd f64 (53 bits) to f32 (24 bits) is then a
///   single correct rounding, including for f32 subnormal results.
///
/// Fast path: `s` is `xy + z` rounded once to f64. Rounding `s` to f32 equals rounding
/// the exact value unless `s` is exactly an f32 rounding midpoint (every midpoint is an
/// f64, so `s` cannot be on the other side of one) or lies in the f32 subnormal range
/// (where the rounding position moves). Zero sums are exact. Only the rare remaining
/// cases take the exact path.
#[inline(always)]
pub fn fmaf_exact(x: f32, y: f32, z: f32) -> f32 {
    let a = f64::from(x) * f64::from(y);
    let b = f64::from(z);
    let s = a + b;
    // One transfer of `s` to integer registers, then integer tests only (a second
    // VFP->ARM transfer for a float compare is costly on the Cortex-A9).
    let bits = s.to_bits();
    let exponent = (bits >> 52) & 0x7ff;
    // f32 normal range or beyond (f64 exponent >= 1023 - 126), including inf/NaN: a
    // midpoint there has f64 mantissa bits 28..0 = 1 followed by 28 zeros.
    // The common case returns first, after one transfer; the zero test (which LLVM
    // turns into a float compare, i.e. a second transfer) only runs on the rare path.
    if exponent >= 897 {
        if bits & 0x1fff_ffff != 0x1000_0000 {
            return s as f32;
        }
    } else if bits << 1 == 0 {
        // A zero sum is exact (xy == -z, or both zero) and carries IEEE's sign rule.
        return s as f32;
    }
    fmaf_exact_slow(a, b, s)
}

#[cold]
#[inline(never)]
fn fmaf_exact_slow(a: f64, b: f64, s: f64) -> f32 {
    if !s.is_finite() {
        return s as f32;
    }
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    let mut bits = s.to_bits();
    if e != 0.0 && bits & 1 == 0 {
        // e != 0 implies s != 0. Increase the magnitude when e points away from zero.
        if (e > 0.0) == (s > 0.0) {
            bits += 1;
        } else {
            bits -= 1;
        }
    }
    f64::from_bits(bits) as f32
}

#[cfg(test)]
mod fmaf_exact_tests {
    use super::fmaf_exact;

    fn check(x: f32, y: f32, z: f32) {
        let want = x.mul_add(y, z);
        let got = fmaf_exact(x, y, z);
        assert!(
            want.to_bits() == got.to_bits() || (want.is_nan() && got.is_nan()),
            "fma({x:e}, {y:e}, {z:e}) = {want:e} ({:#010x}), soft {got:e} ({:#010x})",
            want.to_bits(),
            got.to_bits()
        );
    }

    #[test]
    fn matches_mul_add_on_edge_cases() {
        let specials = [
            0.0f32, -0.0, 1.0, -1.0, 0.5, 2.0, 3.0, 0.1, 1.0 + f32::EPSILON, 1.0 - f32::EPSILON / 2.0,
            f32::MIN_POSITIVE, -f32::MIN_POSITIVE, f32::from_bits(1), f32::from_bits(0x807f_ffff),
            f32::MAX, -f32::MAX, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1e-20, 3e-39, 16777216.0,
        ];
        for &x in &specials {
            for &y in &specials {
                for &z in &specials {
                    check(x, y, z);
                }
            }
        }
    }

    #[test]
    fn matches_mul_add_on_random_and_tie_heavy_inputs() {
        // xorshift, fixed seed: deterministic.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for i in 0..4_000_000u32 {
            let r = next();
            let pick = |bits: u32, mode: u64| -> f32 {
                match mode % 4 {
                    // Any bit pattern.
                    0 => f32::from_bits(bits),
                    // Few significant bits near 1: products that land on float ties.
                    1 => f32::from_bits((bits & 0x807f_0000) | 0x3f00_0000),
                    // Small integers and halves.
                    2 => (bits % 4096) as f32 / 2.0 - 1024.0,
                    // Around the subnormal boundary.
                    _ => f32::from_bits(bits & 0x80ff_ffff),
                }
            };
            let x = pick(r as u32, r >> 32);
            let y = pick((r >> 16) as u32 ^ i, r >> 40);
            let z = pick((next()) as u32, r >> 48);
            check(x, y, z);
        }
    }
}
