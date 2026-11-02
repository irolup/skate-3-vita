//! The Quick Menu ("Properties", FE state 0x36, movie `objectquickmenu`; docs/object-dropper-survey.md "The Quick
//! Menu"): Back opens it in the park editor (`825C9F88`), with or without a selection. Its main list (Color, Branding,
//! Style, Options, Copy, Paste, Undo, Redo; keys `825CF360`), the Options page (`825CE7E0`) and the editor's undo
//! history (six snapshots, `82516F10` / `82517260` / `82517528`) live here; the editor applies what they ask.
//!
//! NOT RETAIL YET: Branding changes nothing (the park brand is not drawn), X on the Style page (apply to the rest of
//! the park) is not done, and Colour's Apply All asks no confirmation.
use super::scene::Scene;

/// The screen modes the C++ posts to the `ObjectQuickMenu` movie (`SetState(n)`, the name table `0x8302C4B8`).
pub mod screen {
    /// Rows 0-7 highlighted: `Enable<row>`.
    pub const ENABLE_ROW: u8 = 0;
    pub const ACTIVATE_OPTIONS: u8 = 8;
    pub const ACTIVATE_QUICK_MENU: u8 = 9;
    pub const ACTIVATE_COLOR: u8 = 10;
    pub const ACTIVATE_STYLE: u8 = 11;
    pub const ACTIVATE_BRANDING: u8 = 12;
    pub const REFRESH_SNAP: u8 = 13;
    pub const REFRESH_COLLISION: u8 = 14;
    pub const REFRESH_INVERT_Y: u8 = 15;
    pub const REFRESH_INVERT_X: u8 = 16;
    pub const REFRESH_CURSOR_SPEED: u8 = 17;
    pub const REFRESH_CURSOR: u8 = 23;
}

/// The main list's rows (`825D02C0`).
pub const COLOR: usize = 0;
pub const BRANDING: usize = 1;
pub const STYLE: usize = 2;
pub const OPTIONS: usize = 3;
pub const COPY: usize = 4;
pub const PASTE: usize = 5;
pub const UNDO: usize = 6;
pub const REDO: usize = 7;
pub const ROWS: usize = 8;

/// The Options page's rows in the C++ order (`Q+0x40`): Snapping, Invert Y, Invert X, Cursor speed, Collision. The
/// movie highlights them by name, so Down from Snapping lands on the third row shown (retail's quirk).
pub const SNAP: usize = 0;
pub const INVERT_Y: usize = 1;
pub const INVERT_X: usize = 2;
pub const CURSOR_SPEED: usize = 3;
pub const COLLISION: usize = 4;
pub const OPTION_ROWS: usize = 5;
/// Each option row's highlight mode (`825CE718`: EnableSnap 18, EnableInvertYAxis 20, EnableInvertXAxis 21,
/// EnableCursor 22, EnableCollision 19).
pub const OPTION_ENABLE: [u8; OPTION_ROWS] = [18, 20, 21, 22, 19];
/// The Style page's rows: the `fe_park_itemtypes` records with an item type, in their order (`82877998`; the
/// movie names them university, industrial, plaza).
pub const STYLES: [&str; 3] = ["university", "industrial", "downtown"];
/// The model retail never tints (vault `terrain_no_tint_models`, `828689F0`).
pub const NO_TINT: &str = "ao_DRPepperCanBalloon";
/// The colour grid's row length (Up / Down step a row, `825CEAF8`).
pub const COLOUR_ROW: usize = 8;
/// The cursor speed's step and range (`0x820641A8`, `0x8209975C`, `0x822249B4`).
pub const SPEED_STEP: f32 = 0.1;
pub const SPEED_RANGE: (f32, f32) = (0.5, 1.5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Main,
    Options,
    Color,
    Branding,
    Style,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quick {
    pub page: Page,
    /// The highlighted row of the main list and of the Options page.
    pub row: usize,
    pub option: usize,
    /// The Colour page's swatch (`Q+0x74`), an index into `Tuning::colours`.
    pub colour: usize,
}

impl Quick {
    /// The next enabled row from the highlighted one, wrapping (`825CE5E0`); the same row when none.
    pub fn step(&self, forward: bool, enabled: &[bool; ROWS]) -> usize {
        (1..=ROWS)
            .map(|k| if forward { (self.row + k) % ROWS } else { (self.row + ROWS * 2 - k) % ROWS })
            .find(|&r| enabled[r])
            .unwrap_or(self.row)
    }
}

/// The undo history (`[0x830670A8]+0x258`): scene snapshots taken before each change, at most six; undo steps back,
/// redo forward, a new change drops what was undone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    states: Vec<Scene>,
    position: usize,
}

/// Snapshots kept (`82516F10`: a push at six drops the oldest).
pub const HISTORY: usize = 6;

impl History {
    /// Before a change: the scene as it is.
    pub fn push(&mut self, scene: &Scene) {
        self.states.truncate(self.position);
        self.states.push(scene.clone());
        if self.states.len() > HISTORY {
            self.states.remove(0);
        }
        self.position = self.states.len();
    }

    pub fn can_undo(&self) -> bool {
        self.position != 0 && self.position <= self.states.len()
    }

    pub fn can_redo(&self) -> bool {
        self.position + 1 < self.states.len()
    }

    /// The scene to go back to (`current` kept for a redo when it is the newest).
    pub fn undo(&mut self, current: &Scene) -> Option<Scene> {
        if !self.can_undo() {
            return None;
        }
        if self.position == self.states.len() {
            self.states.push(current.clone());
        }
        self.position -= 1;
        Some(self.states[self.position].clone())
    }

    pub fn redo(&mut self) -> Option<Scene> {
        if !self.can_redo() {
            return None;
        }
        self.position += 1;
        Some(self.states[self.position].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn rows_skip_the_disabled_ones_and_wrap() {
        let mut enabled = [true; ROWS];
        enabled[COPY] = false;
        enabled[PASTE] = false;
        let q = Quick { row: OPTIONS, ..Default::default() };
        assert_eq!(q.step(true, &enabled), UNDO);
        let q = Quick { row: COLOR, ..Default::default() };
        assert_eq!(q.step(false, &enabled), REDO);
    }

    #[test]
    fn the_history_undoes_and_redoes_six_steps() {
        let mut scene = Scene::default();
        let mut history = History::default();
        assert!(!history.can_undo());
        let mut ids = Vec::new();
        for i in 0..8 {
            history.push(&scene);
            ids.push(scene.drop_item("PROP", Vec3::new(i as f32, 0.0, 0.0), Quat::IDENTITY));
        }
        // Six steps back at most.
        let mut back = 0;
        while let Some(s) = history.undo(&scene) {
            scene = s;
            back += 1;
        }
        assert_eq!((back, scene.items.len()), (6, 2));
        // Forward again to the newest.
        while let Some(s) = history.redo() {
            scene = s;
        }
        assert_eq!(scene.items.len(), 8);
        // Undo once, then a new change: the redo is gone.
        scene = history.undo(&scene).unwrap();
        history.push(&scene);
        scene.drop_item("PROP", Vec3::ZERO, Quat::IDENTITY);
        assert!(!history.can_redo());
    }
}
