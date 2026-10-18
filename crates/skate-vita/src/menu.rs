//! Menus: a list of rows, D-pad to move, left/right to change a value,
//! Cross to choose, Circle to go back. Pure state; drawing is in `ui`.
use vita_platform::controls::sce;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: &'static str,
    /// Shown as `< value >` and changed with left/right.
    pub value: Option<String>,
}
impl Row {
    pub fn action(label: &'static str) -> Self {
        Self { label, value: None }
    }
    pub fn choice(label: &'static str, value: impl Into<String>) -> Self {
        Self { label, value: Some(value.into()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    None,
    Choose(usize),
    Change(usize, i32),
    Back,
}

/// Edge detection with auto-repeat for held directions (menus run at 60 fps).
#[derive(Default)]
pub struct Buttons {
    previous: u32,
    held_frames: u32,
}
impl Buttons {
    /// The buttons that went down this frame, plus held directions repeating.
    pub fn pressed(&mut self, now: u32) -> u32 {
        let edge = now & !self.previous;
        let directions = sce::UP | sce::DOWN | sce::LEFT | sce::RIGHT;
        if now & directions != 0 && now & directions == self.previous & directions {
            self.held_frames += 1;
        } else {
            self.held_frames = 0;
        }
        self.previous = now;
        let repeat = self.held_frames > 24 && self.held_frames % 6 == 0;
        edge | if repeat { now & directions } else { 0 }
    }
}

pub struct Menu {
    pub title: String,
    pub rows: Vec<Row>,
    pub cursor: usize,
}
impl Menu {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), rows: Vec::new(), cursor: 0 }
    }

    /// Applies this frame's pressed buttons; returns what was chosen.
    pub fn input(&mut self, pressed: u32) -> Input {
        if self.rows.is_empty() {
            return Input::None;
        }
        let n = self.rows.len();
        if pressed & sce::UP != 0 {
            self.cursor = (self.cursor + n - 1) % n;
        }
        if pressed & sce::DOWN != 0 {
            self.cursor = (self.cursor + 1) % n;
        }
        self.cursor = self.cursor.min(n - 1);
        if pressed & sce::CIRCLE != 0 {
            return Input::Back;
        }
        if self.rows[self.cursor].value.is_some() {
            if pressed & sce::LEFT != 0 {
                return Input::Change(self.cursor, -1);
            }
            if pressed & sce::RIGHT != 0 {
                return Input::Change(self.cursor, 1);
            }
        }
        if pressed & sce::CROSS != 0 {
            return Input::Choose(self.cursor);
        }
        Input::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_wraps_and_values_change() {
        let mut m = Menu::new("t");
        m.rows = vec![Row::action("A"), Row::choice("B", "x"), Row::action("C")];
        assert_eq!(m.input(sce::UP), Input::None);
        assert_eq!(m.cursor, 2);
        assert_eq!(m.input(sce::RIGHT), Input::None); // no value to change
        assert_eq!(m.input(sce::DOWN | sce::CROSS), Input::Choose(0));
        assert_eq!(m.input(sce::DOWN | sce::RIGHT), Input::Change(1, 1));
        assert_eq!(m.input(sce::CIRCLE), Input::Back);
    }

    #[test]
    fn held_directions_repeat_after_a_delay() {
        let mut b = Buttons::default();
        assert_eq!(b.pressed(sce::DOWN), sce::DOWN);
        let repeats = (0..60).filter(|_| b.pressed(sce::DOWN) != 0).count();
        assert!((5..=7).contains(&repeats), "{repeats}");
        assert_eq!(b.pressed(sce::DOWN | sce::CROSS), sce::CROSS);
    }
}
