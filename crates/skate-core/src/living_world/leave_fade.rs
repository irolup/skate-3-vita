//! How an ambient NPC skater appears and leaves: 1 s fade in, fade out and removal below opacity 0.2.
//! Retail mechanism (82594488, 8246EA90, 8245A9B8, 825926F8): docs/hails-additions/26c-living-world-npc-skaters.md#leave-fade

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
    /// A bailed (simulated) skater fades out before its respawn places it again (retail on: the bail wait `8246EA90`
    /// starts the `+1872` fade; research b715 trace: fade at 4.0 s, re-placed at 5.0 s, no removal).
    pub bail_fade: bool,
    /// How long before the respawn that fade starts, in seconds (retail 1.0).
    pub bail_fade_lead_seconds: f32,
}

impl LeaveFadeConfig {
    pub const RETAIL_FADE_IN_SECONDS: f32 = 1.0;
    pub const RETAIL_FADE_SECONDS: f32 = 1.0;
    pub const RETAIL_DESPAWN_ALPHA: f32 = 0.2;
    pub const RETAIL_BAIL_FADE_LEAD_SECONDS: f32 = 1.0;

    pub fn retail() -> Self {
        Self { fade_in_seconds: Self::RETAIL_FADE_IN_SECONDS, fade_seconds: Self::RETAIL_FADE_SECONDS, despawn_alpha: Self::RETAIL_DESPAWN_ALPHA, bail_fade: true, bail_fade_lead_seconds: Self::RETAIL_BAIL_FADE_LEAD_SECONDS }
    }

    /// Opacity of a bailed skater `ticks` 60 Hz ticks after its bail whose respawn comes at `delay_ticks`: solid until
    /// the lead before the respawn, then the fade out (`1 - t / fade`); solid again once placed (the caller stops
    /// asking). Research b715: docs/hails-additions/26c-living-world-npc-skaters.md#bail-fade
    pub fn bail_alpha(&self, ticks: u64, delay_ticks: u64) -> f32 {
        if !self.bail_fade {
            return 1.0;
        }
        let start = delay_ticks.saturating_sub((self.bail_fade_lead_seconds.max(0.0) * 60.0).round() as u64);
        if ticks < start {
            return 1.0;
        }
        let fade = (self.fade_seconds.max(0.0) * 60.0).round();
        if fade <= 0.0 {
            return 0.0;
        }
        (1.0 - (ticks - start) as f32 / fade as f32).clamp(0.0, 1.0)
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
    /// Start the fade when the cursor has ended at a dead end (stand-in for retail's timed state, fix 9): docs/hails-additions/26c-living-world-npc-skaters.md#leave-fade-start
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

    /// Remove a fading skater below the threshold (8245A9B8), only once the fade in is over: docs/hails-additions/26c-living-world-npc-skaters.md#leave-fade-despawn
    pub fn should_despawn(&self, frames: u64, config: &LeaveFadeConfig) -> bool {
        self.started.is_some() && frames >= config.fade_in_frames() && self.alpha(frames, config) < config.despawn_alpha
    }
}
