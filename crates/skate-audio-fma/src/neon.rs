//! skate_vita (retail-audio, Vita): four channels of the biquad at once with ARMv7 NEON.
//!
//! The Cortex-A9 runs `body::biquad` one sample at a time on a serial dependency chain; four
//! independent channels in the four lanes of a NEON register cost about as much as one. Each lane
//! does exactly what `body::biquad` does under the `no-fma` arithmetic: NEON's `vmla.f32` is an
//! unfused multiply then add (two roundings, as `a * b + c`), the feed-forward association follows
//! the sample's position `n % 8` (the same for every lane), and the feedback is `(−a1)·y1` then
//! `(−a2)·y2`. NEON flushes denormals to zero, which the scalar VFP path in IEEE mode does not, so a
//! denormal input or state may differ (the kernel's 1e-18 bias keeps the state out of that range).
use crate::{BIAS, Coefficients};
use core::arch::arm::*;

/// `samples[i]` with history `history[i]`, for four channels of the same length (a multiple of 4).
///
/// # Safety
/// Needs NEON (the caller is compiled for `target_feature = "neon"`).
pub unsafe fn biquad4(k: &Coefficients, history: [&mut [f32; 4]; 4], samples: [&mut [f32]; 4]) {
    let len = samples[0].len();
    debug_assert!(samples.iter().all(|s| s.len() == len) && len % 4 == 0);
    // SAFETY (whole body): NEON is available (see above); every load and store below reads or
    // writes four floats inside `samples[c][n..n + 4]` with `n + 4 <= len`.
    unsafe {
        let lane = |i: usize| -> float32x4_t {
            let v = [history[0][i], history[1][i], history[2][i], history[3][i]];
            vld1q_f32(v.as_ptr())
        };
        let (mut x1, mut x2, mut y1, mut y2) = (lane(0), lane(1), lane(2), lane(3));
        let b0 = vdupq_n_f32(k.b0);
        let b1 = vdupq_n_f32(k.b1);
        let b2 = vdupq_n_f32(k.b2);
        let na1 = vdupq_n_f32(-k.a1);
        let na2 = vdupq_n_f32(-k.a2);
        let bias = vdupq_n_f32(BIAS);
        let [s0, s1, s2, s3] = samples;
        let mut n = 0;
        while n < len {
            // Four samples of each channel, transposed to four samples × four channels.
            let q0 = vld1q_f32(s0.as_ptr().add(n));
            let q1 = vld1q_f32(s1.as_ptr().add(n));
            let q2 = vld1q_f32(s2.as_ptr().add(n));
            let q3 = vld1q_f32(s3.as_ptr().add(n));
            let a = vtrnq_f32(q0, q1);
            let b = vtrnq_f32(q2, q3);
            let xs = [
                vcombine_f32(vget_low_f32(a.0), vget_low_f32(b.0)),
                vcombine_f32(vget_low_f32(a.1), vget_low_f32(b.1)),
                vcombine_f32(vget_high_f32(a.0), vget_high_f32(b.0)),
                vcombine_f32(vget_high_f32(a.1), vget_high_f32(b.1)),
            ];
            let mut ys = [vdupq_n_f32(0.0); 4];
            for (i, &x) in xs.iter().enumerate() {
                // vmlaq_f32(acc, a, b) = acc + a·b, unfused: `a.madd(b, acc)` under no-fma.
                let t = match (n + i) % 8 {
                    0 => vmlaq_f32(vmlaq_f32(vmlaq_f32(bias, b0, x), b2, x2), b1, x1),
                    1 => vmlaq_f32(vmlaq_f32(vmlaq_f32(bias, b0, x), b1, x1), b2, x2),
                    _ => vmlaq_f32(vmlaq_f32(vmlaq_f32(bias, b1, x1), b2, x2), b0, x),
                };
                let y = vmlaq_f32(vmlaq_f32(t, na1, y1), na2, y2);
                x2 = x1;
                x1 = x;
                y2 = y1;
                y1 = y;
                ys[i] = y;
            }
            // Back to four channels × four samples.
            let a = vtrnq_f32(ys[0], ys[1]);
            let b = vtrnq_f32(ys[2], ys[3]);
            vst1q_f32(s0.as_mut_ptr().add(n), vcombine_f32(vget_low_f32(a.0), vget_low_f32(b.0)));
            vst1q_f32(s1.as_mut_ptr().add(n), vcombine_f32(vget_low_f32(a.1), vget_low_f32(b.1)));
            vst1q_f32(s2.as_mut_ptr().add(n), vcombine_f32(vget_high_f32(a.0), vget_high_f32(b.0)));
            vst1q_f32(s3.as_mut_ptr().add(n), vcombine_f32(vget_high_f32(a.1), vget_high_f32(b.1)));
            n += 4;
        }
        let mut out = [[0.0f32; 4]; 4];
        for (i, v) in [x1, x2, y1, y2].into_iter().enumerate() {
            let mut lanes = [0.0f32; 4];
            vst1q_f32(lanes.as_mut_ptr(), v);
            for c in 0..4 {
                out[c][i] = lanes[c];
            }
        }
        for (h, o) in history.into_iter().zip(out) {
            *h = o;
        }
    }
}
