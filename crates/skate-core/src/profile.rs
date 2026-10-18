//! skate_vita addition: dependency-free stage timers for profiling the simulation step.
//! With the `stage-profile` feature off (the default) `scope` returns a unit guard and
//! compiles to nothing. Timing never changes any computation.

#[cfg(feature = "stage-profile")]
mod imp {
    use std::sync::Mutex;
    use std::time::Instant;

    static TOTALS: Mutex<Vec<(&'static str, u64, u64)>> = Mutex::new(Vec::new());

    pub struct Scope {
        name: &'static str,
        start: Instant,
    }

    #[inline]
    pub fn scope(name: &'static str) -> Scope {
        Scope { name, start: Instant::now() }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            let nanos = self.start.elapsed().as_nanos() as u64;
            let mut totals = TOTALS.lock().unwrap();
            match totals.iter_mut().find(|t| t.0 == self.name) {
                Some(t) => {
                    t.1 += nanos;
                    t.2 += 1;
                }
                None => totals.push((self.name, nanos, 1)),
            }
        }
    }

    /// Adds `value` to a named counter (e.g. rows per tick).
    pub fn count(name: &'static str, value: u64) {
        let mut totals = TOTALS.lock().unwrap();
        match totals.iter_mut().find(|t| t.0 == name) {
            Some(t) => {
                t.1 += value;
                t.2 += 1;
            }
            None => totals.push((name, value, 1)),
        }
    }

    /// (name, total nanoseconds or counter sum, entries); resets.
    pub fn take() -> Vec<(&'static str, u64, u64)> {
        std::mem::take(&mut *TOTALS.lock().unwrap())
    }
}

#[cfg(not(feature = "stage-profile"))]
mod imp {
    /// Zero-sized guard, so callers can `drop` it like the timed one.
    pub struct Scope;
    #[inline(always)]
    pub fn scope(_: &'static str) -> Scope {
        Scope
    }
    #[inline(always)]
    pub fn count(_: &'static str, _: u64) {}
    pub fn take() -> Vec<(&'static str, u64, u64)> {
        Vec::new()
    }
}

pub use imp::{count, scope, take};
