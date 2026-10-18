//! skate_vita diagnostic (feature `profile`, retail-audio S0): accumulated time per render stage,
//! for the Vita benchmark. Without the feature every scope is empty and costs nothing.

pub const NAMES: [&str; 12] = [
    "grain_bed", "voice_resample", "voice_filters", "voice_sends", "voice_gain", "voice_pan", "voice_out",
    "bus_submix", "bus_flange", "bus_env", "bus_eq", "eval_block",
];

#[cfg(feature = "profile")]
mod imp {
    use std::sync::Mutex;
    use std::time::Instant;
    pub static TOTALS: Mutex<[u64; 12]> = Mutex::new([0; 12]);
    pub struct Scope(usize, Instant);
    impl Scope {
        #[inline]
        pub fn new(i: usize) -> Self {
            Scope(i, Instant::now())
        }
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            TOTALS.lock().unwrap()[self.0] += self.1.elapsed().as_nanos() as u64;
        }
    }
    /// The accumulated nanoseconds per stage since the last call.
    pub fn take() -> [u64; 12] {
        std::mem::take(&mut *TOTALS.lock().unwrap())
    }
}
#[cfg(not(feature = "profile"))]
mod imp {
    pub struct Scope;
    impl Scope {
        #[inline(always)]
        pub fn new(_: usize) -> Self {
            Scope
        }
    }
    pub fn take() -> [u64; 12] {
        [0; 12]
    }
}
pub use imp::{Scope, take};
