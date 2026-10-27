//! Retail's loading screen (TipsLoader, FE screen 52, movie `screens/main/tiploader`): which panel shows, which
//! line it says and how long it stays. Details: docs/hails-additions/31-retail-menus.md#loading-screen-fe-52

/// The panels in the movie's order (`Is<X>Enabled(i)`, byte T+196+i).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    CoachFrank = 0,
    Stats = 1,
    Hint = 2,
    Leaderboard = 3,
    Feed = 4,
    Reel = 5,
}

pub const PANELS: [Panel; 6] = [Panel::CoachFrank, Panel::Stats, Panel::Hint, Panel::Leaderboard, Panel::Feed, Panel::Reel];

/// Seconds a panel shows (0x8211F0F8) and its outro takes (0x8209975C) before the next pick (sub_82605CC0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub show: f32,
    pub outro: f32,
}

impl Timing {
    pub const RETAIL: Self = Self { show: 9.0, outro: 0.5 };
}

impl Default for Timing {
    fn default() -> Self {
        Self::RETAIL
    }
}

/// The movie's panel state (tiploader.apt kState_Invisible 0 / Populate 1 / Outro 2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Invisible,
    Populate(Panel),
    Outro(Panel),
}

impl State {
    pub fn code(self) -> u8 {
        match self {
            State::Invisible => 0,
            State::Populate(_) => 1,
            State::Outro(_) => 2,
        }
    }
}

/// The tips object's pick memory (T+196.. last shown, T+202.. shown before, T+208.. counts) and the panel cycle.
/// Kept per boot like retail's globals; one per client.
#[derive(Clone, Debug)]
pub struct TipsLoader {
    last_shown: [bool; 6],
    shown_before: [bool; 6],
    counts: [u32; 6],
    /// The previous Coach Frank / hint line (0x830CEBDC / 0x830CEBE0, per boot; 0 before the first pick).
    last_coach: usize,
    last_hint: usize,
    state: State,
    timer: f32,
    pub timing: Timing,
}

impl Default for TipsLoader {
    fn default() -> Self {
        Self { last_shown: [false; 6], shown_before: [false; 6], counts: [0; 6], last_coach: 0, last_hint: 0, state: State::Invisible, timer: 0.0, timing: Timing::RETAIL }
    }
}

/// A line of `n` other than the previous one, uniform (sub_82606C98 / sub_82605FA8): `rng % (n - 1)`, shifted past
/// the previous. One line or none: line 0, and the previous stays as it was (retail writes it only on a pick).
pub fn pick_line(n: usize, last: &mut usize, rng: &mut dyn FnMut() -> u32) -> usize {
    if n <= 1 {
        return 0;
    }
    let i = rng() as usize % (n - 1);
    let line = if i >= *last { i + 1 } else { i };
    *last = line;
    line
}

impl TipsLoader {
    pub fn state(&self) -> State {
        self.state
    }

    /// The panel picked last (`Is<X>Enabled`: byte T+196+i), if any.
    pub fn shown(&self) -> Option<Panel> {
        self.last_shown.iter().position(|&s| s).map(|i| PANELS[i])
    }

    /// The panel to show next among the `available` ones (sub_82608598): the least shown first (count equal to the
    /// smallest of all six counts), then the ones shown more, then the ones flagged shown before, then the one shown
    /// last; uniform within that group. None when no panel is available.
    pub fn pick_panel(&mut self, available: [bool; 6], rng: &mut dyn FnMut() -> u32) -> Option<Panel> {
        let min = self.counts.iter().copied().min().unwrap_or(0);
        let mut buckets: [Vec<usize>; 4] = Default::default();
        for i in (0..6).filter(|&i| available[i]) {
            let bucket = if self.last_shown[i] {
                3
            } else if self.shown_before[i] {
                2
            } else if self.counts[i] == min {
                0
            } else {
                1
            };
            buckets[bucket].push(i);
        }
        let bucket = buckets.iter().find(|b| !b.is_empty())?;
        let i = bucket[rng() as usize % bucket.len()];
        self.last_shown = [false; 6];
        self.last_shown[i] = true;
        self.shown_before[i] = true;
        self.counts[i] += 1;
        Some(PANELS[i])
    }

    pub fn coach_line(&mut self, n: usize, rng: &mut dyn FnMut() -> u32) -> usize {
        pick_line(n, &mut self.last_coach, rng)
    }

    pub fn hint_line(&mut self, n: usize, rng: &mut dyn FnMut() -> u32) -> usize {
        pick_line(n, &mut self.last_hint, rng)
    }

    /// The screen opened (its data is ready offline at once): back to the first pick, and no panel counts as shown
    /// before (sub_82605AA0 clears T+202..207 on every open; the counts and the last shown panel stay, research b588).
    pub fn open(&mut self) {
        self.state = State::Invisible;
        self.timer = 0.0;
        self.shown_before = [false; 6];
    }

    /// One update of `dt` seconds while the screen is up (sub_82605CC0): a pick when invisible, the outro after
    /// `timing.show`, the next pick after `timing.outro`. Returns the new state when it changed.
    pub fn update(&mut self, dt: f32, available: [bool; 6], rng: &mut dyn FnMut() -> u32) -> Option<State> {
        self.timer += dt;
        let next = match self.state {
            State::Invisible => self.pick_panel(available, rng).map(State::Populate),
            State::Populate(p) if self.timer >= self.timing.show => Some(State::Outro(p)),
            State::Outro(_) if self.timer >= self.timing.outro => self.pick_panel(available, rng).map(State::Populate),
            _ => None,
        }?;
        self.state = next;
        self.timer = 0.0;
        Some(next)
    }
}

/// What the load screen asks of the front end (the stage carries it out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadStep {
    /// Hold the fader black (normal rate).
    HoldBlack,
    /// Push FE 52 (LoadScreenManager kind 2 open, sub_825E6F30).
    Push,
    /// Release the black at once: the screen is up (82605CA4).
    ReleaseInstant,
    /// Close FE 52 (kind 2 close, sub_825E7058); the fader is black.
    Close,
    /// Release the black (normal rate): the fade back in after the black frames.
    Release,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoadPhase {
    #[default]
    Idle,
    ToBlack,
    Opening,
    Up,
    Closing,
    Closed,
    BlackFrames(u32),
}

/// FE updates of full black after the update that sees the screen gone, before the fade back in: none, so the
/// release comes 2 updates after the close. Retail's hold state (sub_82708D00) waits for 6 ticks of another
/// thread's counter; the recomp shows 2 to 3 FE updates from close to release (research b588).
pub const BLACK_FRAMES: u32 = 0;

/// The loading screen around a load (game state 0x82707508 with sub_826E08A8 and LoadScreenManager kind 2; order from
/// the recomp trace): fade to black, push the screen at black, drop the black at once when it is up, and when the load
/// is done fade to black, close it, hold black a few updates, fade back in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadScreen {
    pub phase: LoadPhase,
}

impl LoadScreen {
    /// The screen is black or the loading screen is up: the world behind it may change.
    pub fn covered(&self) -> bool {
        !matches!(self.phase, LoadPhase::Idle | LoadPhase::ToBlack)
    }

    /// Any part of the sequence runs.
    pub fn active(&self) -> bool {
        self.phase != LoadPhase::Idle
    }

    /// One FE update: `loading` = a load runs, `black` = the fader is fully black, `up` = the screen's intro is
    /// done, `gone` = the screen closed.
    pub fn update(&mut self, loading: bool, black: bool, up: bool, gone: bool) -> Option<LoadStep> {
        let (phase, step) = match self.phase {
            LoadPhase::Idle if loading => (LoadPhase::ToBlack, Some(LoadStep::HoldBlack)),
            LoadPhase::ToBlack if black => (LoadPhase::Opening, Some(LoadStep::Push)),
            LoadPhase::Opening if up => (LoadPhase::Up, Some(LoadStep::ReleaseInstant)),
            LoadPhase::Up if !loading => (LoadPhase::Closing, Some(LoadStep::HoldBlack)),
            LoadPhase::Closing if black => (LoadPhase::Closed, Some(LoadStep::Close)),
            LoadPhase::Closed if gone => (LoadPhase::BlackFrames(BLACK_FRAMES), None),
            LoadPhase::BlackFrames(0) => (LoadPhase::Idle, Some(LoadStep::Release)),
            LoadPhase::BlackFrames(n) => (LoadPhase::BlackFrames(n - 1), None),
            phase => (phase, None),
        };
        self.phase = phase;
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(values: &[u32]) -> impl FnMut() -> u32 + '_ {
        let mut i = 0;
        move || {
            let v = values[i % values.len()];
            i += 1;
            v
        }
    }

    const OFFLINE: [bool; 6] = [true, false, true, false, false, false];

    #[test]
    fn a_line_is_never_the_previous_one() {
        let mut last = 3;
        for r in 0..20 {
            let mut rng = || r;
            let line = pick_line(5, &mut last.clone(), &mut rng);
            assert_ne!(line, 3);
            assert!(line < 5);
        }
        assert_eq!(pick_line(5, &mut last, &mut || 3), 4, "an index at or past the previous one moves up one");
        assert_eq!(last, 4);
        assert_eq!(pick_line(1, &mut last, &mut || 7), 0);
        assert_eq!(pick_line(0, &mut last, &mut || 7), 0);
        assert_eq!(last, 4, "one line or none leaves the previous one");
    }

    /// Every open clears "shown before" (sub_82605AA0), so the least-shown rule keeps working load after load.
    #[test]
    fn each_open_forgets_shown_before_but_keeps_the_counts() {
        let mut t = TipsLoader::default();
        let mut rng = seq(&[0]);
        assert_eq!(t.pick_panel(OFFLINE, &mut rng), Some(Panel::CoachFrank));
        t.open();
        // Coach Frank is no longer "shown before" but was shown more: the hint (count 0) still goes first.
        assert_eq!(t.pick_panel(OFFLINE, &mut rng), Some(Panel::Hint));
        t.open();
        let with_stats = [true, true, true, false, false, false];
        assert_eq!(t.pick_panel(with_stats, &mut rng), Some(Panel::Stats), "the never shown panel wins");
    }

    #[test]
    fn panels_go_least_shown_first_and_never_twice_in_a_row_while_another_is_free() {
        let mut t = TipsLoader::default();
        let mut rng = seq(&[0]);
        let first = t.pick_panel(OFFLINE, &mut rng).unwrap();
        assert_eq!(first, Panel::CoachFrank, "both unshown: uniform over them, index 0");
        assert_eq!(t.pick_panel(OFFLINE, &mut rng), Some(Panel::Hint), "Coach Frank was shown before: the hint goes first");
        // Both shown before now; the last shown (hint) is the last resort.
        assert_eq!(t.pick_panel(OFFLINE, &mut rng), Some(Panel::CoachFrank));
        assert_eq!(t.pick_panel([false, false, true, false, false, false], &mut rng), Some(Panel::Hint), "the only panel shows again");
        assert_eq!(t.pick_panel([false; 6], &mut rng), None);
    }

    #[test]
    fn stats_join_when_available_and_unshown_panels_win() {
        let mut t = TipsLoader::default();
        let mut rng = seq(&[0]);
        t.pick_panel(OFFLINE, &mut rng);
        t.pick_panel(OFFLINE, &mut rng);
        let with_stats = [true, true, true, false, false, false];
        assert_eq!(t.pick_panel(with_stats, &mut rng), Some(Panel::Stats));
    }

    #[test]
    fn a_load_fades_out_shows_the_screen_and_fades_back_in() {
        let mut l = LoadScreen::default();
        assert_eq!(l.update(false, false, false, false), None);
        assert_eq!(l.update(true, false, false, false), Some(LoadStep::HoldBlack));
        assert!(!l.covered(), "fading out: the world still shows");
        assert_eq!(l.update(true, false, false, false), None);
        assert_eq!(l.update(true, true, false, false), Some(LoadStep::Push));
        assert!(l.covered());
        assert_eq!(l.update(false, true, false, false), None, "a load that ended meanwhile waits for the screen");
        assert_eq!(l.update(false, true, true, false), Some(LoadStep::ReleaseInstant));
        assert_eq!(l.update(false, false, true, false), Some(LoadStep::HoldBlack));
        assert_eq!(l.update(false, false, true, false), None);
        assert_eq!(l.update(false, true, true, false), Some(LoadStep::Close));
        assert_eq!(l.update(false, true, false, true), None);
        for _ in 0..BLACK_FRAMES {
            assert_eq!(l.update(false, true, false, true), None);
        }
        assert_eq!(l.update(false, true, false, true), Some(LoadStep::Release));
        assert!(!l.active());
    }

    #[test]
    fn the_cycle_shows_a_panel_then_its_outro_then_the_next() {
        let mut t = TipsLoader::default();
        let mut rng = seq(&[0]);
        t.open();
        assert_eq!(t.update(0.016, OFFLINE, &mut rng), Some(State::Populate(Panel::CoachFrank)));
        assert_eq!(t.update(8.9, OFFLINE, &mut rng), None);
        assert_eq!(t.update(0.2, OFFLINE, &mut rng), Some(State::Outro(Panel::CoachFrank)));
        assert_eq!(t.update(0.4, OFFLINE, &mut rng), None);
        assert_eq!(t.update(0.2, OFFLINE, &mut rng), Some(State::Populate(Panel::Hint)));
        assert_eq!(t.state().code(), 1);
        t.open();
        assert_eq!(t.state(), State::Invisible);
    }
}
