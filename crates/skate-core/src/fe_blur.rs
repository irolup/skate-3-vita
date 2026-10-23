//! Front-end backdrop blur and tint: the world behind a menu screen blurs and darkens while that screen is on top
//! of the FE screen stack (doc 31, "Pause blur").
//!
//! Retail (TU3 recomp addresses): every FE screen has a record in the screen stack's info table (0x83026088,
//! 72 records of 24 bytes, byte +4 = blur). The stack applies the flag of the screen on top on every push / pop
//! through vt+28 = sub_825EBED8, which stores it at FE manager +461 and, on an off -> on change only, restarts the
//! ramp (tint 255, blur 0.0). An empty stack switches it off. The FE manager tick sub_825D8E68 then ramps the blur
//! strength (+468 / +472) and the RGB tint (+476 / +480 / +484) one step per tick, up when on and back when off.
//! The values here are the retail defaults; mods override them per screen (menu_backdrop).

/// Ramp parameters (retail defaults in `RETAIL`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeBlurParams {
    /// Full blur strength (0x82099250 = 8.0).
    pub strength: f32,
    /// Blur change per tick (0x8208824C = 0.8, ten ticks to full).
    pub step: f32,
    /// Tint floor while on (immediates in sub_825D8E68: 90, 85, 81).
    pub tint: [i32; 3],
    /// Tint change per tick (15).
    pub tint_step: i32,
    /// Ticks per second of the FE manager (the console's 30 fps cadence; not measured).
    pub tick_hz: f32,
}

impl FeBlurParams {
    pub const RETAIL: Self = Self { strength: 8.0, step: 0.8, tint: [90, 85, 81], tint_step: 15, tick_hz: 30.0 };
}

impl Default for FeBlurParams {
    fn default() -> Self {
        Self::RETAIL
    }
}

/// FE manager blur state (+461 enable, +468 / +472 strength, +476.. tint).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeBlur {
    pub enabled: bool,
    pub strength: f32,
    pub tint: [i32; 3],
    /// Seconds not yet spent on a tick.
    pending: f32,
}

impl Default for FeBlur {
    fn default() -> Self {
        Self { enabled: false, strength: 0.0, tint: [255; 3], pending: 0.0 }
    }
}

impl FeBlur {
    /// sub_825EBED8: store the flag; an off -> on change restarts the ramp from no blur and no tint.
    pub fn set_enabled(&mut self, enable: bool) {
        if enable && !self.enabled {
            self.strength = 0.0;
            self.tint = [255; 3];
        }
        self.enabled = enable;
    }

    /// One FE manager tick (sub_825D8E68), with retail's clamps: on, the tint falls to its floor and the strength
    /// rises to full; off, both go back (tint 255, strength 0).
    pub fn tick(&mut self, p: &FeBlurParams) {
        if self.enabled {
            for (c, floor) in self.tint.iter_mut().zip(p.tint) {
                *c -= p.tint_step;
                if *c <= floor {
                    *c = floor;
                }
            }
            self.strength += p.step;
            if self.strength >= p.strength {
                self.strength = p.strength;
            }
        } else {
            for c in &mut self.tint {
                *c += p.tint_step;
                if *c >= 255 {
                    *c = 255;
                }
            }
            self.strength -= p.step;
            if self.strength <= 0.0 {
                self.strength = 0.0;
            }
        }
    }

    /// Run the ticks due after `seconds` at the console cadence, so any frame rate gives the same ramp.
    pub fn advance(&mut self, seconds: f32, p: &FeBlurParams) {
        let period = 1.0 / p.tick_hz.max(1.0);
        self.pending = (self.pending + seconds.max(0.0)).min(period * 64.0);
        while self.pending >= period {
            self.pending -= period;
            self.tick(p);
        }
    }

    /// Anything to draw (blur above zero or a tint below white).
    pub fn visible(&self) -> bool {
        self.strength > 0.0 || self.tint != [255; 3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_up_and_back_like_retail() {
        let p = FeBlurParams::RETAIL;
        let mut b = FeBlur::default();
        b.set_enabled(true);
        for _ in 0..9 {
            b.tick(&p);
        }
        assert!(b.strength < 8.0 && b.tint == [120, 120, 120]);
        b.tick(&p);
        b.tick(&p);
        assert_eq!((b.strength, b.tint), (8.0, [90, 90, 90]));
        b.tick(&p);
        assert_eq!(b.tint, [90, 85, 81]);
        b.set_enabled(false);
        for _ in 0..12 {
            b.tick(&p);
        }
        assert_eq!((b.strength, b.tint), (0.0, [255; 3]));
        assert!(!b.visible());
    }

    #[test]
    fn re_enabling_restarts_only_from_off() {
        let p = FeBlurParams::RETAIL;
        let mut b = FeBlur::default();
        b.set_enabled(true);
        b.tick(&p);
        b.tick(&p);
        b.set_enabled(true);
        assert_eq!(b.tint, [225; 3], "on -> on keeps the ramp");
        b.set_enabled(false);
        b.tick(&p);
        b.set_enabled(true);
        assert_eq!((b.strength, b.tint), (0.0, [255; 3]), "off -> on restarts it");
    }

    #[test]
    fn same_ramp_at_any_frame_rate() {
        let p = FeBlurParams::RETAIL;
        let (mut slow, mut fast) = (FeBlur::default(), FeBlur::default());
        slow.set_enabled(true);
        fast.set_enabled(true);
        for _ in 0..15 {
            slow.advance(1.0 / 30.0 + 1e-6, &p);
        }
        for _ in 0..60 {
            fast.advance(1.0 / 120.0 + 1e-6, &p);
        }
        assert_eq!((slow.strength, slow.tint), (fast.strength, fast.tint));
    }
}
