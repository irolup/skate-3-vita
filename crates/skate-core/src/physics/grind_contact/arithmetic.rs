//! Refinement sequence used by S3 82D861D8/82D88518/82D886B8.
//! Seeds use existing independent PC math, not a Xenon bit-exact claim.
use crate::madd::Madd as _;
use crate::physics::native_arithmetic;

pub(super) fn reciprocal(value: f32) -> f32 {
    let mut result = native_arithmetic::reciprocal_estimate(value);
    for _ in 0..2 {
        let error = (-result).madd(value, 1.0);
        result = result.madd(error, result);
    }
    result
}

pub(super) fn inverse_square_root(value: f32) -> f32 {
    let mut result = native_arithmetic::reciprocal_square_root_estimate(value);
    for _ in 0..2 {
        let square = result * result;
        let half = result * 0.5;
        let error = (-value).madd(square, 1.0);
        result = half.madd(error, result);
    }
    result
}

pub(super) fn square_root(value: f32) -> f32 {
    let result = value * inverse_square_root(value);
    // The original vector select explicitly chooses zero for a zero radicand.
    if value == 0.0 { 0.0 } else { result }
}
