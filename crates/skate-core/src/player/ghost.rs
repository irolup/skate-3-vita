//! Skater fade-in after every placement ("ghost"), TU3 sub_82594488.
//!
//! Retail keeps a fade-in timer in seconds per skater placement component (owner +1868). The
//! place-skater handler sub_825926F8 sets it to 0 (and counts the placement, +1864); the
//! component constructor sub_82590DC0 starts it at 0 too, so a fresh skater fades in. Each tick
//! sub_82594488(owner, dt) publishes the opacity (record +190 = opacity * 255, read by the
//! character shaders as `i_params.x`) and then adds dt:
//! - timer < 1: opacity = clamp(timer, 0, 1), and the timer stays put while state byte 71 is
//!   set and timer > 0.68 (0x82099764),
//! - else a fade-out timer (+1872, constructor FLT_MAX = off) would apply; nothing in our port
//!   starts it,
//! - else 1.0.
//!
//! Generalised for mods: opacity = clamp(timer / fade_in_seconds, 0, 1) and the hold compares
//! the opacity with `hold_alpha` (retail 1.0 s and 0.68, where both forms are the same).
//! Time-based, so the curve is the same at any tick rate.
//!
//! NOT RETAIL YET: who sets state byte 71 (the 0.68 hold) is undecoded; hosts pass `hold =
//! false`.

/// Longest fade a mod may ask for (s); also where the timer stops counting.
pub const MAX_FADE_IN_SECONDS: f32 = 60.0;

/// Fade tuning (`ghost` world tuning domain). `default()` = retail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GhostSettings {
    /// Fade in after placements at all (retail true).
    pub enabled: bool,
    /// Seconds from placement to fully opaque (retail 1.0, sub_82594488 compares timer < 1.0).
    pub fade_in_seconds: f32,
    /// Opacity the fade waits at while the hold condition is set (retail 0.68, 0x82099764).
    pub hold_alpha: f32,
}

impl Default for GhostSettings {
    fn default() -> Self {
        Self { enabled: true, fade_in_seconds: 1.0, hold_alpha: 0.68 }
    }
}

impl GhostSettings {
    pub fn valid(&self) -> bool {
        self.fade_in_seconds.is_finite()
            && (0.0..=MAX_FADE_IN_SECONDS).contains(&self.fade_in_seconds)
            && self.hold_alpha.is_finite()
            && (0.0..=1.0).contains(&self.hold_alpha)
    }
}

/// One skater's fade-in timer (retail owner +1868).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GhostFade {
    /// Seconds since the last placement (stops at [`MAX_FADE_IN_SECONDS`]).
    pub timer: f32,
    /// Opacity published by the last [`Self::step`] (1 = solid).
    pub opacity: f32,
}

impl Default for GhostFade {
    /// Constructor sub_82590DC0: timer 0, so a spawned skater fades in.
    fn default() -> Self {
        Self { timer: 0.0, opacity: 0.0 }
    }
}

impl GhostFade {
    /// Place-skater sub_825926F8: restart the fade.
    pub fn place(&mut self) {
        self.timer = 0.0;
    }

    /// Opacity for a timer value (no state change).
    pub fn opacity_at(timer: f32, settings: &GhostSettings) -> f32 {
        if !settings.enabled || settings.fade_in_seconds <= 0.0 || timer >= settings.fade_in_seconds {
            return 1.0;
        }
        (timer / settings.fade_in_seconds).clamp(0.0, 1.0)
    }

    /// One tick of sub_82594488: publish the opacity, then advance the timer by `dt` seconds
    /// unless `hold` is set and the opacity is past `hold_alpha`.
    pub fn step(&mut self, dt: f32, settings: &GhostSettings, hold: bool) -> f32 {
        self.opacity = Self::opacity_at(self.timer, settings);
        let held = hold && self.opacity < 1.0 && self.opacity > settings.hold_alpha;
        if !held {
            self.timer = (self.timer + dt.max(0.0)).min(MAX_FADE_IN_SECONDS);
        }
        self.opacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn retail_curve_is_linear_over_one_second() {
        let s = GhostSettings::default();
        let mut f = GhostFade::default();
        assert_eq!(f.step(DT, &s, false), 0.0, "placement tick draws at 0");
        let mut last = 0.0;
        for tick in 1..60 {
            let a = f.step(DT, &s, false);
            assert!(a > last && a < 1.0, "tick {tick}: {a}");
            assert!((a - tick as f32 * DT).abs() < 1e-4, "tick {tick}: {a}");
            last = a;
        }
        // Float sums of 1/60 land within one tick of 1.0 s.
        let a = f.step(DT, &s, false);
        assert!(a == 1.0 || a > 0.999, "{a}");
        assert_eq!(f.step(DT, &s, false), 1.0);
    }

    #[test]
    fn frame_rate_independent() {
        let s = GhostSettings::default();
        let (mut a, mut b) = (GhostFade::default(), GhostFade::default());
        for _ in 0..30 {
            a.step(1.0 / 60.0, &s, false);
        }
        for _ in 0..15 {
            b.step(1.0 / 30.0, &s, false);
        }
        assert!((a.timer - b.timer).abs() < 1e-5);
        assert!((GhostFade::opacity_at(a.timer, &s) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn place_restarts_and_disabled_is_solid() {
        let s = GhostSettings::default();
        let mut f = GhostFade::default();
        for _ in 0..120 {
            f.step(DT, &s, false);
        }
        assert_eq!(f.opacity, 1.0);
        f.place();
        assert_eq!(f.step(DT, &s, false), 0.0);
        let off = GhostSettings { enabled: false, ..s };
        f.place();
        assert_eq!(f.step(DT, &off, false), 1.0);
        let instant = GhostSettings { fade_in_seconds: 0.0, ..s };
        f.place();
        assert_eq!(f.step(DT, &instant, false), 1.0);
    }

    #[test]
    fn hold_waits_past_hold_alpha() {
        let s = GhostSettings::default();
        let mut f = GhostFade::default();
        for _ in 0..600 {
            f.step(DT, &s, true);
        }
        assert!(f.opacity > 0.68 && f.opacity < 0.70, "{}", f.opacity);
        for _ in 0..60 {
            f.step(DT, &s, false);
        }
        assert_eq!(f.opacity, 1.0, "released hold finishes the fade");
    }

    #[test]
    fn settings_validation() {
        assert!(GhostSettings::default().valid());
        assert!(!GhostSettings { fade_in_seconds: -1.0, ..Default::default() }.valid());
        assert!(!GhostSettings { fade_in_seconds: f32::NAN, ..Default::default() }.valid());
        assert!(!GhostSettings { hold_alpha: 1.5, ..Default::default() }.valid());
    }
}
