//! How an ambient NPC skater appears and leaves: it fades in over its first second, and at the end
//! it fades out and the skater manager removes it once the fade is far enough along, instead of
//! popping in or vanishing on the spot.
//!
//! Retail [code, TU3]:
//! - The skater's render object (`skater+1804`, a sub-object at `+14960` of the 15328-byte object
//!   built by `sub_82B973C8`, vtable `0x8231E170`) keeps an opacity at `+244`: vfunc 68 sets it,
//!   vfunc 76 (`sub_82B97190`) returns it. Default 1.0.
//! - `sub_82594488(skater, dt)` drives it from two timers: `skater+1868` (fade in, opacity =
//!   clamp(t, 0, 1)) and `skater+1872` (fade out, opacity = 1 - clamp(t, 0, 1)); both grow by dt
//!   every tick, so a fade takes 1 s. `+1872` = FLT_MAX means "not fading out" (`sub_8246EF78`).
//! - `sub_8246EA90` (ambient skater controller) starts the fade out (`+1872 = 0`, controller byte
//!   `+29 = 1`) once the time in its timed state is more than `duration - 1 s`: the fade ends
//!   exactly when the state does.
//! - The manager's per-skater check `sub_8245A9B8` removes a skater whose controller is fading
//!   (`+29`) once the opacity (vfunc 76) is below 0.2 (constant at `0x82099280`), 0.8 s into the
//!   fade.
//! - `sub_825926F8` (skater spawn setup) sets `+1868 = 0`, so every spawned skater fades in from
//!   opacity 0 to 1 over 1 s. `sub_82594488` checks the fade in first: while `+1868 < 1` the opacity
//!   is the fade-in value even if a fade out was started (held at 0.5 while a component byte `+71`
//!   is set; not ported, the replay tier has no such component).
//!
//! That timed state is entered on a skater component flag (`sub_8246EF78`, byte `+59`), not at a
//! line end: at a line end retail chains to the next line (`replay::choose_next_line`, fix 9), so
//! an ambient skater keeps riding until the 120 m population cull. The replay tier fades only at a
//! dead end (no line starts within the chain radius): the fade starts there with the last pose
//! held, the NPC is removed below `despawn_alpha`.
//! Pure and deterministic (a function of the cursor frame), so a client derives the same value.

use super::replay::{LineCursor, LineSource};

/// Data-driven spawn fade-in and leave fade (retail values as defaults; mods may override).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeaveFadeConfig {
    /// Length of the fade in after a spawn in seconds (retail: the `+1868` timer runs 0 to 1 at dt
    /// per tick, `sub_825926F8` / `sub_82594488`). 0 = appear at once.
    pub fade_in_seconds: f32,
    /// Length of the fade out in seconds (retail: the `+1872` timer runs 0 to 1 at dt per tick).
    pub fade_seconds: f32,
    /// Opacity below which the manager removes a fading skater (retail 0.2, `sub_8245A9B8`).
    pub despawn_alpha: f32,
}

impl LeaveFadeConfig {
    pub const RETAIL_FADE_IN_SECONDS: f32 = 1.0;
    pub const RETAIL_FADE_SECONDS: f32 = 1.0;
    pub const RETAIL_DESPAWN_ALPHA: f32 = 0.2;

    pub fn retail() -> Self {
        Self { fade_in_seconds: Self::RETAIL_FADE_IN_SECONDS, fade_seconds: Self::RETAIL_FADE_SECONDS, despawn_alpha: Self::RETAIL_DESPAWN_ALPHA }
    }

    /// Fade-in length in 60 Hz frames (0 = no fade in).
    pub fn fade_in_frames(&self) -> u64 {
        (self.fade_in_seconds.max(0.0) * 60.0).round() as u64
    }

    /// Opacity of the spawn fade in, `frames` 60 Hz frames after the spawn: clamp(t / fade, 0, 1)
    /// (`sub_82594488`, timer `+1868`).
    pub fn fade_in_alpha(&self, frames: u64) -> f32 {
        match self.fade_in_frames() {
            0 => 1.0,
            n => (frames as f32 / n as f32).clamp(0.0, 1.0),
        }
    }

    /// Fade length in 60 Hz frames (at least one frame).
    pub fn fade_frames(&self) -> u64 {
        ((self.fade_seconds.max(0.0) * 60.0).round() as u64).max(1)
    }
}

impl Default for LeaveFadeConfig {
    fn default() -> Self {
        Self::retail()
    }
}

/// Fade-out state of one NPC skater: the cursor frame the fade started on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeaveFade {
    pub started: Option<u64>,
}

/// 60 Hz frames until the cursor's line ends, or `None` while a branch group is still ahead (a
/// branch may continue the ride) or the line is unknown.
pub fn frames_to_line_end(cursor: &LineCursor, lines: &dyn LineSource) -> Option<u64> {
    if cursor.finished {
        return Some(0);
    }
    let line = lines.line(&cursor.line)?;
    if line.groups.iter().any(|g| g.node > cursor.node) {
        return None;
    }
    let ahead: u64 = (cursor.node..line.nodes.len().saturating_sub(1) as u32).map(|i| u64::from(line.segment_frames(i))).sum();
    Some(ahead.saturating_sub(u64::from(cursor.frame_in_segment)))
}

impl LeaveFade {
    /// Start the fade when the cursor has ended: its line is over and no other line starts
    /// within the chain radius (`replay::choose_next_line`). Returns true on the frame it starts.
    ///
    /// Retail (fix 9): the 1 s fade and the removal at opacity 0.2 belong to a timed state the
    /// controller enters on a skater component flag (`sub_8246EF78`, byte `+59`; leaving it
    /// cancels the fade), not to the line end: at a line end retail chains to the next line
    /// (`sub_8246C7F8`). The replay tier has no such state; its only stand-in is a dead end the
    /// replay cannot ride past, where the fade starts with the last pose held.
    pub fn update(&mut self, cursor: &LineCursor, _lines: &dyn LineSource, _config: &LeaveFadeConfig) -> bool {
        if self.started.is_some() || !cursor.finished {
            return false;
        }
        self.started = Some(cursor.frames);
        true
    }

    /// Opacity `frames` 60 Hz frames after the spawn (`sub_82594488`): the fade in while it runs
    /// (it wins over a fade out, like retail), then 1 until the fade out starts, then 1 - t / fade.
    pub fn alpha(&self, frames: u64, config: &LeaveFadeConfig) -> f32 {
        if frames < config.fade_in_frames() {
            return config.fade_in_alpha(frames);
        }
        self.fade_out_alpha(frames, config)
    }

    fn fade_out_alpha(&self, frames: u64, config: &LeaveFadeConfig) -> f32 {
        match self.started {
            None => 1.0,
            Some(s) => {
                let t = frames.saturating_sub(s) as f32 / config.fade_frames() as f32;
                1.0 - t.clamp(0.0, 1.0)
            }
        }
    }

    /// The manager removes a fading skater below the threshold (`sub_8245A9B8`). Checked once the
    /// fade in is over: retail's timed state lasts at least 1.5 s (`sub_8246EE30`), so its fade out
    /// never starts while the fade in is still below 0.5, and a skater is never removed mid fade
    /// in; our lines can be shorter, so the rule is stated explicitly.
    pub fn should_despawn(&self, frames: u64, config: &LeaveFadeConfig) -> bool {
        self.started.is_some() && frames >= config.fade_in_frames() && self.alpha(frames, config) < config.despawn_alpha
    }
}
