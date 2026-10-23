//! skate_vita (R2 performance): time spent in each phase of the front end's frame, behind the `fe-profile` feature
//! (without it `span` is an empty guard and [`take`] is empty). Spans nest: each total includes the spans inside it.
#[derive(Clone, Copy, Debug)]
pub enum Span {
    /// MenuStage: the FrontEnd and its registry view, built every frame.
    FrontEnd,
    /// MenuStage: the keys and the stack's effects (movie calls they make included).
    Keys,
    Effects,
    /// MenuStage: the registry work after the movies (selects, steps, settings refresh).
    Registry,
    /// Stage::update, and in it: queued loads, the levels' timelines, the action queue, the garbage collection.
    StageUpdate,
    Loads,
    Advance,
    Seek,
    Drain,
    /// In the action queue: copying an action block, running it.
    ActionCopy,
    Run,
    /// Host natives (Game, CoreMenu, FELanguage, ...), inside the runs.
    Natives,
    Collect,
    /// MenuStage::draw (the scene walk) and its post-processing.
    Draw,
}

const NAMES: [&str; 15] = [
    "front_end", "keys", "effects", "registry", "stage_update", "loads", "advance", "seek", "drain", "action_copy", "run",
    "natives", "collect", "draw", "-",
];

#[cfg(feature = "fe-profile")]
mod imp {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    pub static NANOS: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];
    pub static COUNTS: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];
    pub static ALLOCS: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];
    /// A test's allocation counter (0 = none).
    pub static HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    pub fn allocations_now() -> u64 {
        allocations()
    }
    fn allocations() -> u64 {
        match HOOK.load(Ordering::Relaxed) {
            0 => 0,
            f => unsafe { std::mem::transmute::<usize, fn() -> u64>(f)() },
        }
    }

    pub struct Guard(usize, Instant, u64);
    impl Drop for Guard {
        fn drop(&mut self) {
            NANOS[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed);
            COUNTS[self.0].fetch_add(1, Ordering::Relaxed);
            ALLOCS[self.0].fetch_add(allocations() - self.2, Ordering::Relaxed);
        }
    }
    pub fn span(s: super::Span) -> Guard {
        Guard(s as usize, Instant::now(), allocations())
    }
}

#[cfg(feature = "fe-profile")]
pub use imp::Guard;
#[cfg(not(feature = "fe-profile"))]
pub struct Guard;

/// Times the rest of the scope under `s`.
#[inline(always)]
pub fn span(s: Span) -> Guard {
    #[cfg(feature = "fe-profile")]
    return imp::span(s);
    #[cfg(not(feature = "fe-profile"))]
    {
        let _ = s;
        Guard
    }
}

/// Installs an allocation counter whose growth the spans record (tests).
pub fn set_allocation_counter(f: fn() -> u64) {
    #[cfg(feature = "fe-profile")]
    imp::HOOK.store(f as usize, std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(feature = "fe-profile"))]
    let _ = f;
}

/// (phase, milliseconds, entries, allocations) since the last call, phases with entries only; resets the totals.
pub fn take() -> Vec<(&'static str, f64, u64, u64)> {
    #[cfg(feature = "fe-profile")]
    {
        use std::sync::atomic::Ordering;
        return (0..NAMES.len())
            .filter_map(|i| {
                let n = imp::COUNTS[i].swap(0, Ordering::Relaxed);
                let t = imp::NANOS[i].swap(0, Ordering::Relaxed);
                let a = imp::ALLOCS[i].swap(0, Ordering::Relaxed);
                (n > 0).then(|| (NAMES[i], t as f64 / 1e6, n, a))
            })
            .collect();
    }
    #[cfg(not(feature = "fe-profile"))]
    Vec::new()
}
