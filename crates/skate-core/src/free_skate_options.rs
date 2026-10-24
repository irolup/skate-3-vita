//! Free Play options (retail FE 34 FreeSkateOption, template 13 `main/freeskate_options.swf`, research b548 / b564):
//! Traffic, Pedestrians and A.I. Skaters on a working copy of the Free Play values; B writes them back.
//! Details: docs/hails-additions/31-retail-menus.md#free-play-options-fe-34

use crate::living_world::config::FreePlay;

#[derive(Clone, Debug, PartialEq)]
pub struct FreeSkateRules {
    /// Left / Right step (0.1), the top (1.0) and the snap to 0 below it on the way down (0.09) (8260A4E8).
    pub step: f32,
    pub max: f32,
    pub snap: f32,
    pub sound_up: String,
    pub sound_down: String,
    pub sound_back: String,
    /// The movie's state that marks the selected row (SetState 999); rows 0..2 refresh themselves (SetState row).
    pub state_selection: i32,
}

impl Default for FreeSkateRules {
    fn default() -> Self {
        Self { step: 0.1, max: 1.0, snap: 0.09, sound_up: "core_nav_up".into(), sound_down: "core_nav_down".into(), sound_back: "core_b_button".into(), state_selection: 999 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Back,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub sound: Option<String>,
    pub states: Vec<i32>,
    /// B: the values to write back (flow 0x8B), then the screen closes.
    pub close: Option<FreePlay>,
}

/// The screen's working copy (init copies the globals 0x830B7C34 / C38 / C3C).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FreeSkateOptions {
    pub values: FreePlay,
    /// Row 0 Traffic, 1 Pedestrians, 2 A.I. Skaters (screen +0x150).
    pub row: usize,
}

pub const ROWS: usize = 3;

impl FreeSkateOptions {
    pub fn open(values: FreePlay) -> Self {
        Self { values, row: 0 }
    }

    /// `GetTrafficSetting` / `GetPedestrianSetting` (825AD680 / 825AD6F8): tenths, `trunc(v * 10 + 0.1)`, 0 below 1.
    pub fn tenths(v: f32) -> i32 {
        let t = v * 10.0 + 0.1;
        if t < 1.0 { 0 } else { t as i32 }
    }

    /// C++ input 8260A4E8.
    pub fn key(&mut self, key: Key, rules: &FreeSkateRules) -> Outcome {
        let step = |v: &mut f32, up: bool| {
            *v = if up { (*v + rules.step).min(rules.max) } else { *v - rules.step };
            if *v < rules.snap {
                *v = 0.0;
            }
        };
        match key {
            Key::Up | Key::Down => {
                self.row = if key == Key::Down { (self.row + 1) % ROWS } else { (self.row + ROWS - 1) % ROWS };
                let sound = if key == Key::Down { &rules.sound_down } else { &rules.sound_up };
                Outcome { sound: Some(sound.clone()), states: vec![rules.state_selection], close: None }
            }
            Key::Left | Key::Right => {
                let up = key == Key::Right;
                match self.row {
                    0 => step(&mut self.values.traffic, up),
                    1 => step(&mut self.values.pedestrians, up),
                    _ => self.values.ai_skaters = !self.values.ai_skaters,
                }
                Outcome { sound: None, states: vec![self.row as i32], close: None }
            }
            Key::Back => {
                let mut v = self.values;
                for x in [&mut v.traffic, &mut v.pedestrians] {
                    if *x < rules.snap {
                        *x = 0.0;
                    }
                }
                Outcome { sound: Some(rules.sound_back.clone()), states: Vec::new(), close: Some(v) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenths_have_exact_band_edges() {
        assert_eq!(FreeSkateOptions::tenths(0.0), 0);
        assert_eq!(FreeSkateOptions::tenths(0.05), 0);
        assert_eq!(FreeSkateOptions::tenths(0.3), 3);
        assert_eq!(FreeSkateOptions::tenths(0.7 - 1e-6), 7);
        assert_eq!(FreeSkateOptions::tenths(1.0), 10);
    }

    #[test]
    fn rows_wrap_steps_clamp_and_back_writes() {
        let rules = FreeSkateRules::default();
        let mut s = FreeSkateOptions::open(FreePlay::default());
        assert_eq!(s.key(Key::Up, &rules).states, [999]);
        assert_eq!(s.row, 2);
        s.key(Key::Left, &rules);
        assert!(!s.values.ai_skaters);
        s.key(Key::Down, &rules);
        assert_eq!(s.row, 0);
        s.key(Key::Right, &rules);
        assert_eq!(s.values.traffic, 1.0);
        for _ in 0..10 {
            s.key(Key::Left, &rules);
        }
        assert_eq!(s.values.traffic, 0.0);
        assert_eq!(FreeSkateOptions::tenths(s.values.traffic), 0);
        let out = s.key(Key::Back, &rules);
        assert_eq!(out.close, Some(FreePlay { traffic: 0.0, pedestrians: 1.0, ai_skaters: false }));
        assert_eq!(out.sound.as_deref(), Some("core_b_button"));
    }
}
