//! Lowers the two caches that fill while skating when they no longer fit beside the world. Full, the world replay
//! holds 24 MiB and the decoded animation keys 16 MiB; DownTown holds about 260 MB before them, and 20 KB failed in the
//! 304 MiB heap (319 MB) with 305 MB held, 70 s after a load. Both give memory back without changing the game: a
//! replay gets shorter, and a released animation key decodes to the same words when it is next needed.

/// Bytes held (large and small blocks) above which the caches shrink. Besides malloc's overhead (3-4%) the reserved
/// region and area keep memory idle: 256 KB was refused at 284.6 MB held, with a Challenge Map teleport's tiles coming in.
const LIMIT: usize = 262_000_000;
/// The most a check takes off a budget (a check a second): a replay's eviction runs on the simulation thread.
const STEP: usize = 1 << 20;
/// About 10 s of replay in a busy DownTown (2.9 s per MB).
const MIN_REPLAY: usize = 4 << 20;
const MIN_ANIMATION: usize = 8 << 20;

/// The caches' budgets, from the room the rest of the game leaves them. The room is what counts, not what the caches
/// hold at the moment: going by that, pressure early in a session (both nearly empty) left the replay at its minimum
/// for good.
pub struct Budgets {
    pub replay: usize,
    pub animation: usize,
    full: (usize, usize),
}

impl Budgets {
    pub fn new(replay: usize, animation: usize) -> Self {
        Self { replay, animation, full: (replay, animation) }
    }

    /// One check with what is held now and what each cache holds. True when a budget changed. The room is shared in
    /// proportion to the full budgets, a budget down by steps and up at once (a cache only grows into it slowly).
    pub fn update(&mut self, held: usize, replay_bytes: usize, animation_bytes: usize) -> bool {
        let room = LIMIT.saturating_sub(held.saturating_sub(replay_bytes + animation_bytes));
        let share = (room as u64 * self.full.1 as u64 / (self.full.0 + self.full.1) as u64) as usize;
        let animation = share.clamp(MIN_ANIMATION, self.full.1);
        let replay = room.saturating_sub(animation).clamp(MIN_REPLAY, self.full.0);
        // What the replay's maximum leaves over goes to the animation keys.
        let animation = room.saturating_sub(replay).clamp(MIN_ANIMATION, self.full.1);
        let next = |now: usize, target: usize| {
            if target < now {
                now.saturating_sub(STEP).max(target)
            } else if target >= now + STEP {
                target
            } else {
                now
            }
        };
        let (r, a) = (next(self.replay, replay), next(self.animation, animation));
        let changed = (r, a) != (self.replay, self.animation);
        (self.replay, self.animation) = (r, a);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: usize = 1 << 20;

    #[test]
    fn with_room_the_budgets_stay_full() {
        let mut b = Budgets::new(24 * MB, 16 * MB);
        assert!(!b.update(200_000_000, 24 * MB, 16 * MB));
        assert_eq!((b.replay, b.animation), (24 * MB, 16 * MB));
    }

    #[test]
    fn a_budget_goes_down_by_steps_and_stops_at_its_minimum() {
        let mut b = Budgets::new(24 * MB, 16 * MB);
        // The rest of the game holds more than the limit: only the minimums are left.
        assert!(b.update(LIMIT + 50 * MB, 24 * MB, 16 * MB));
        assert_eq!((b.replay, b.animation), (23 * MB, 15 * MB));
        for _ in 0..40 {
            b.update(LIMIT + 50 * MB, b.replay, b.animation);
        }
        assert_eq!((b.replay, b.animation), (MIN_REPLAY, MIN_ANIMATION));
    }

    #[test]
    fn pressure_early_in_a_session_does_not_stay() {
        let mut b = Budgets::new(24 * MB, 16 * MB);
        for _ in 0..40 {
            b.update(LIMIT + 5 * MB, 2 * MB, 2 * MB);
        }
        assert_eq!((b.replay, b.animation), (MIN_REPLAY, MIN_ANIMATION));
        // The world holds less later: the budgets come back at once, with the caches still small.
        assert!(b.update(200_000_000, 3 * MB, 3 * MB));
        assert_eq!((b.replay, b.animation), (24 * MB, 16 * MB));
    }

    #[test]
    fn downtown_settles_at_the_limit_with_the_rest_for_the_replay() {
        let world = 247_000_000;
        let mut b = Budgets::new(24 * MB, 16 * MB);
        let (mut replay, mut animation) = (0usize, 0usize);
        for _ in 0..600 {
            replay = (replay + 330_000).min(b.replay);
            animation = (animation + 250_000).min(b.animation);
            b.update(world + replay + animation, replay, animation);
        }
        assert!(world + replay + animation < LIMIT + 2 * MB);
        assert_eq!(animation, MIN_ANIMATION);
        assert!((6 * MB..9 * MB).contains(&replay), "{replay}");
    }
}
