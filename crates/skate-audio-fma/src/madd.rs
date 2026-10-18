//! skate_vita (as `skate_core::madd`): every `.mul_add(` of skate-audio and of this
//! crate's loops is `.madd(`. By default `madd` is `mul_add` (the reference's exactly rounded
//! a·b + c); with the `no-fma` feature it is a separate multiply and add, the Vita's arithmetic:
//! the Cortex-A9 has no fused multiply-add, and newlib's `fmaf` is a slow double-precision call.

pub trait Madd {
    /// `self * a + b`.
    fn madd(self, a: Self, b: Self) -> Self;
}

impl Madd for f32 {
    #[inline(always)]
    fn madd(self, a: f32, b: f32) -> f32 {
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
