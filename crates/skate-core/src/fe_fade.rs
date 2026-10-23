//! The front end's fade to black (doc 31 "Fade to black"): retail's FE fader object (`[[0x830CFE14]+304]`, TU3).
//! A screen change holds a bit: the alpha rises 0.04 per FE tick, and when the screen is black the waiting actions
//! run (hold sub_825E6B30, tick sub_825E69A0). Releasing the last bit fades back in at 0.04 per tick (release
//! sub_825E6BF8; with its instant flag in one tick). Alpha 1 = black. Values are data (`FeFadeParams`), retail's
//! the defaults.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeFadeParams {
    /// Alpha change per tick towards black (0x82216FEC = 0.04, 25 ticks).
    pub step: f32,
    /// Alpha change per tick back (0x822F88F8 = -0.04).
    pub release_step: f32,
    /// FE ticks per second. NOT RETAIL YET: the console's 30 Hz cadence is assumed (as fe_blur).
    pub tick_hz: f32,
}

impl FeFadeParams {
    pub const RETAIL: Self = Self { step: 0.04, release_step: -0.04, tick_hz: 30.0 };
}

impl Default for FeFadeParams {
    fn default() -> Self {
        Self::RETAIL
    }
}

/// The fader: alpha, its step, the hold bits and the actions waiting for black.
#[derive(Clone, Debug, PartialEq)]
pub struct FeFade<A> {
    pub alpha: f32,
    step: f32,
    holds: u32,
    waiting: Vec<A>,
    /// Seconds not yet spent on a tick.
    carry: f32,
}

impl<A> Default for FeFade<A> {
    fn default() -> Self {
        Self { alpha: 0.0, step: 0.0, holds: 0, waiting: Vec::new(), carry: 0.0 }
    }
}

impl<A> FeFade<A> {
    /// sub_825E6B30: hold `bit`; `action` runs when the screen is black (now, if it already is). `instant` goes black
    /// in one tick. Returns the action when it is due at once.
    pub fn hold(&mut self, bit: u32, action: Option<A>, instant: bool, p: &FeFadeParams) -> Option<A> {
        self.holds |= 1 << bit;
        if self.alpha >= 1.0 {
            return action;
        }
        self.waiting.extend(action);
        self.step = if instant { 1.0 } else { p.step };
        None
    }

    /// sub_825E6BF8: release `bit`; with no holds left the screen fades back in (`instant`: in one tick).
    pub fn release(&mut self, bit: u32, instant: bool, p: &FeFadeParams) {
        self.holds &= !(1 << bit);
        if self.holds == 0 && (self.alpha > 0.0 || self.step > 0.0) {
            self.step = if instant { -1.0 } else { p.release_step };
        }
    }

    pub fn held(&self, bit: u32) -> bool {
        self.holds & (1 << bit) != 0
    }

    /// One tick (sub_825E69A0): move the alpha; at black the waiting actions are returned (in order).
    pub fn tick(&mut self) -> Vec<A> {
        if self.step == 0.0 {
            return Vec::new();
        }
        self.alpha += self.step;
        if self.alpha <= 0.0 {
            self.alpha = 0.0;
            self.step = 0.0;
        } else if self.alpha >= 1.0 {
            self.alpha = 1.0;
            self.step = 0.0;
            return std::mem::take(&mut self.waiting);
        }
        Vec::new()
    }

    /// The ticks due after `seconds` at the console cadence (any frame rate gives the same timing).
    pub fn advance(&mut self, seconds: f32, p: &FeFadeParams) -> Vec<A> {
        let period = 1.0 / p.tick_hz.max(1.0);
        self.carry = (self.carry + seconds.max(0.0)).min(period * 30.0);
        let mut due = Vec::new();
        while self.carry >= period {
            self.carry -= period;
            due.extend(self.tick());
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_runs_the_action_at_black_and_release_fades_back() {
        let p = FeFadeParams::RETAIL;
        let mut f = FeFade::default();
        assert_eq!(f.hold(0, Some("open"), false, &p), None);
        let mut ticks = 0;
        let fired = loop {
            ticks += 1;
            let due = f.tick();
            if !due.is_empty() {
                break due;
            }
        };
        assert_eq!((fired, ticks, f.alpha), (vec!["open"], 25, 1.0));
        // Already black: the next hold's action is due at once.
        assert_eq!(f.hold(1, Some("again"), false, &p), Some("again"));
        f.release(0, false, &p);
        f.tick();
        assert_eq!(f.alpha, 1.0, "bit 1 still holds");
        f.release(1, false, &p);
        for _ in 0..25 {
            f.tick();
        }
        assert_eq!(f.alpha, 0.0);
    }

    #[test]
    fn instant_and_cadence() {
        let p = FeFadeParams::RETAIL;
        let mut f: FeFade<()> = FeFade::default();
        f.hold(0, None, true, &p);
        f.tick();
        assert_eq!(f.alpha, 1.0);
        f.release(0, true, &p);
        f.tick();
        assert_eq!(f.alpha, 0.0);
        // 25 ticks at 30 Hz: black after 25 / 30 s whatever the frame time.
        f.hold(0, None, false, &p);
        for _ in 0..50 {
            f.advance(1.0 / 60.0, &p);
        }
        assert!((f.alpha - 1.0).abs() < 1e-5 || f.alpha > 0.99, "{}", f.alpha);
    }
}
