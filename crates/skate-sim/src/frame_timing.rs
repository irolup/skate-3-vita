//! skate-game's frame timing (upstream PR #52): the physics phase clock behind the `FRAME_HITCH`
//! log line, a PC diagnostic. Here the phases run with no clock.

pub(crate) mod hitch {
    pub(crate) const PHASE_ANIM_GRAPHS: usize = 0;
    pub(crate) const PHASE_SOLVE: usize = 1;
    pub(crate) const PHASE_SOLVE_NPC: usize = 4;
    pub(crate) const PHASE_FINISH: usize = 2;
    pub(crate) const PHASE_SCORING: usize = 3;

    /// Runs `f` (upstream also adds its wall time to the phase).
    pub(crate) fn phase<R>(_phase: usize, f: impl FnOnce() -> R) -> R {
        f()
    }

    /// Upstream adds the time since `started` to the phase.
    pub(crate) fn add_phase(_phase: usize, _started: std::time::Instant) {}
}
