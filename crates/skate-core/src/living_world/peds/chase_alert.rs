//! The alert over a ped that warns or chases the player: retail's "ChaseArrow" HUD (b191).
//!
//! The graph op `SendChaseStateMessage` (`826A77A8`) posts {ped, state} for a player target; the HUD's handler
//! (`825E35F0`) keeps at most 10 entries: a new ped is added (not for "end"), a known ped's state is replaced and its
//! timer restarts, "end" removes it. The update (`825E2738`) ages them (warn 6 s, giveup 3 s, chase and tired stay),
//! drops a ped that is gone, projects the ped and scales the sprite by camera distance. The render (`825E2B80`)
//! draws a 64 px sprite per state over the ped when it is inside the centred 0.85 box, else an edge arrow on a ring
//! round the screen centre. Engine-independent: the host feeds states and positions and draws [`Placement`]s.

/// Retail's chase states (`SendChaseStateMessage stateName`, `826A76B8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChaseState {
    Warn,
    Chase,
    Tired,
    Giveup,
    /// `end` (any other name): removes the ped's alert.
    End,
}

impl ChaseState {
    /// The graph's state name (warn, chase, tired, giveup; anything else is end).
    pub fn from_name(name: &str) -> Self {
        match name {
            "warn" => Self::Warn,
            "chase" => Self::Chase,
            "tired" => Self::Tired,
            "giveup" => Self::Giveup,
            _ => Self::End,
        }
    }
    /// Stable key for textures and mods.
    pub fn key(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Chase => "chase",
            Self::Tired => "tired",
            Self::Giveup => "giveup",
            Self::End => "end",
        }
    }
}

/// Retail values as defaults; data and mods may change them.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertSettings {
    /// Entries at once (`825E35F0`: inline storage of 10).
    pub max_entries: usize,
    /// Warn lifetime in ms (`825E2738`: 6000).
    pub warn_ms: u32,
    /// Giveup lifetime in ms (6000 - 3000).
    pub giveup_ms: u32,
    /// Sprite size in px at scale 1 (half-width 32.0 at `0x8302F188`, height 64.0 at `0x822F95B0`).
    pub size: f32,
    /// Scale 1.0 up to `near` m, `far_scale` from `far` m, linear between (1 m, 50 m, 0.4).
    pub near: f32,
    pub far: f32,
    pub far_scale: f32,
    /// On-screen box: inside `margin` of half the screen each way round the centre (manager +0x90 = 0.85).
    pub margin: f32,
    /// Edge ring: `margin * width * 0.5 + ring_extra` px from the centre (+20).
    pub ring_extra: f32,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self { max_entries: 10, warn_ms: 6000, giveup_ms: 3000, size: 64.0, near: 1.0, far: 50.0, far_scale: 0.4, margin: 0.85, ring_extra: 20.0 }
    }
}

impl AlertSettings {
    /// `825E2738`: by camera distance, 1.0 near, `far_scale` far, linear between.
    pub fn scale(&self, distance: f32) -> f32 {
        if distance < self.near {
            1.0
        } else if distance < self.far {
            self.far_scale + (self.far - distance) / (self.far - self.near) * (1.0 - self.far_scale)
        } else {
            self.far_scale
        }
    }
    fn lifetime(&self, state: ChaseState) -> Option<u32> {
        match state {
            ChaseState::Warn => Some(self.warn_ms),
            ChaseState::Giveup => Some(self.giveup_ms),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub ped: u64,
    pub state: ChaseState,
    pub ms: u32,
}

/// The alert list (`825E35F0` / `825E2738`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Alerts {
    pub entries: Vec<Entry>,
}

impl Alerts {
    /// One state message for `ped`.
    pub fn post(&mut self, settings: &AlertSettings, ped: u64, state: ChaseState) {
        match (self.entries.iter().position(|e| e.ped == ped), state) {
            (Some(i), ChaseState::End) => {
                self.entries.remove(i);
            }
            (Some(i), _) => self.entries[i] = Entry { ped, state, ms: 0 },
            (None, ChaseState::End) => {}
            (None, _) if self.entries.len() < settings.max_entries => self.entries.push(Entry { ped, state, ms: 0 }),
            (None, _) => {}
        }
    }
    /// Age every entry by `ms`; drop expired ones and peds for which `alive` is false.
    pub fn tick(&mut self, settings: &AlertSettings, ms: u32, alive: impl Fn(u64) -> bool) {
        self.entries.retain_mut(|e| {
            if !alive(e.ped) {
                return false;
            }
            // `825E2738`: kept while `ms + dt < limit` (no limit for chase and tired).
            let aged = e.ms.saturating_add(ms);
            if settings.lifetime(e.state).is_some_and(|limit| aged >= limit) {
                return false;
            }
            e.ms = aged;
            true
        });
    }
}

/// Where one alert goes on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// Over the ped: the anchor's screen point (px) and the sprite scale.
    Over { at: [f32; 2], scale: f32 },
    /// At the screen edge: the ring point (px) and the angle (radians, clockwise from screen-up) toward the ped.
    Edge { at: [f32; 2], angle: f32, scale: f32 },
}

/// `825E2738` / `825E2B80`: `projected` = the anchor's screen point when it is in front of the camera.
pub fn place(settings: &AlertSettings, projected: Option<[f32; 2]>, behind_dir: [f32; 2], screen: [f32; 2], distance: f32) -> Placement {
    let scale = settings.scale(distance);
    let centre = [screen[0] * 0.5, screen[1] * 0.5];
    if let Some(p) = projected {
        let inside = (p[0] - centre[0]).abs() <= settings.margin * centre[0] && (p[1] - centre[1]).abs() <= settings.margin * centre[1];
        if inside {
            return Placement::Over { at: p, scale };
        }
    }
    // Toward the ped: from the projection when in front, else the given direction (screen space, y down).
    let dir = match projected {
        Some(p) => [p[0] - centre[0], p[1] - centre[1]],
        None => behind_dir,
    };
    let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-6);
    let dir = [dir[0] / len, dir[1] / len];
    let radius = settings.margin * screen[0] * 0.5 + settings.ring_extra;
    let at = [centre[0] + dir[0] * radius, centre[1] + dir[1] * radius];
    Placement::Edge { at, angle: dir[0].atan2(-dir[1]), scale }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_replace_restart_and_end_like_retail() {
        let s = AlertSettings::default();
        let mut a = Alerts::default();
        a.post(&s, 7, ChaseState::Warn);
        a.tick(&s, 4000, |_| true);
        a.post(&s, 7, ChaseState::Chase);
        assert_eq!(a.entries, [Entry { ped: 7, state: ChaseState::Chase, ms: 0 }]);
        a.tick(&s, 60_000, |_| true);
        assert_eq!(a.entries.len(), 1, "chase has no time limit");
        a.post(&s, 7, ChaseState::End);
        assert!(a.entries.is_empty());
        a.post(&s, 8, ChaseState::End);
        assert!(a.entries.is_empty(), "end never adds");
    }

    #[test]
    fn warn_lasts_six_seconds_giveup_three_and_ten_at_most() {
        let s = AlertSettings::default();
        let mut a = Alerts::default();
        a.post(&s, 1, ChaseState::Warn);
        a.post(&s, 2, ChaseState::Giveup);
        a.tick(&s, 2999, |_| true);
        assert_eq!(a.entries.len(), 2);
        a.tick(&s, 1, |_| true);
        assert_eq!(a.entries.iter().map(|e| e.ped).collect::<Vec<_>>(), [1], "giveup gone at 3 s");
        a.tick(&s, 2999, |_| true);
        assert_eq!(a.entries.len(), 1);
        a.tick(&s, 1, |_| true);
        assert!(a.entries.is_empty(), "warn gone at 6 s");
        for ped in 0..12 {
            a.post(&s, ped, ChaseState::Chase);
        }
        assert_eq!(a.entries.len(), 10);
        a.tick(&s, 16, |ped| ped != 3);
        assert_eq!(a.entries.len(), 9, "a ped that is gone loses its alert");
    }

    #[test]
    fn scale_and_placement_follow_retail() {
        let s = AlertSettings::default();
        assert_eq!(s.scale(0.5), 1.0);
        assert!((s.scale(1.0) - 1.0).abs() < 1e-6);
        assert!((s.scale(25.5) - 0.7).abs() < 1e-6);
        assert_eq!(s.scale(80.0), 0.4);
        let screen = [1280.0, 720.0];
        assert_eq!(place(&s, Some([640.0, 300.0]), [0.0, 0.0], screen, 10.0), Placement::Over { at: [640.0, 300.0], scale: s.scale(10.0) });
        let Placement::Edge { at, angle, .. } = place(&s, Some([1270.0, 360.0]), [0.0, 0.0], screen, 10.0) else { panic!() };
        assert!((at[0] - (640.0 + 0.85 * 640.0 + 20.0)).abs() < 1e-3 && (at[1] - 360.0).abs() < 1e-3);
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "points right");
        let Placement::Edge { angle, .. } = place(&s, None, [0.0, 1.0], screen, 10.0) else { panic!() };
        assert!((angle - std::f32::consts::PI).abs() < 1e-5, "behind and below: points down");
    }
}
