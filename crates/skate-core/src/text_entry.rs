//! Text entry (doc 31 "Extras and cheat codes"): retail FE screen 46 (vtable 0x82304CD8, research b305) and the
//! on-screen keyboard that stands in for the Xbox system keyboard it opens.
//!
//! Retail FE 46 has no movie: init sets "show keyboard", the update calls `XamShowKeyboardUI` with the title, the
//! default text and max_len + 1, and the result goes through the OK path 0x82639890:
//! - empty text and `allow_empty` off: the keyboard opens again;
//! - in modes 0 and 1 every character must be in FELanguage `ALL_VALID_ALPHANUMERIC_CHARACTERS`, else the warning
//!   popup (FE popup record 0x107: ID_COMMON_WARNING_TITLE / ID_VIRTUALKEYBOARD_ALPHA_NUMERIC_STRING_ERROR /
//!   ID_COMMON_OK) and the keyboard opens again after OK;
//! - valid: the caller's callback gets the text as typed (no case change), and FE 46 pops;
//! - cancel pops FE 46 (the cheat row passes no cancel callback).
//!
//! The keyboard itself ([`Keyboard`]: a letter grid for the pad, typing for a PC keyboard) is ours: the user chose an
//! on-screen keyboard (2026-10-19); the layout and keys are data (`KeyboardLayout`).

/// One entry request (`sub_826397D8`'s arguments; the cheat row's values in [`TextEntryRequest::cheat_code`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEntryRequest {
    /// The keyboard title (string id or `#literal text`).
    pub title: String,
    pub default: String,
    /// Characters kept (`+0x644`, the cheat row 0x200 = 512).
    pub max_len: usize,
    /// `+0x64D`: an empty entry goes to the callback (else the keyboard opens again).
    pub allow_empty: bool,
    /// Modes 0 and 1 check the characters against the allowed set; modes 2 and up do not.
    pub mode: u32,
    /// Who gets the text (a stable id: `cheat_code`, or `mod:<mod>:<key>`).
    pub callback: String,
}

impl TextEntryRequest {
    /// Extras row 0 (`sub_826558E8`): the row's label as title, empty default, mode 0, 512, empty allowed.
    pub fn cheat_code(title: &str) -> Self {
        Self { title: title.into(), default: String::new(), max_len: 0x200, allow_empty: true, mode: 0, callback: CHEAT_CALLBACK.into() }
    }
}

pub const CHEAT_CALLBACK: &str = "cheat_code";

/// The OK path's verdict (0x82639890).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryResult {
    /// Give the text to the callback and close.
    Accepted(String),
    /// Empty and not allowed: keep the keyboard open.
    Reopen,
    /// Characters outside the allowed set: show the warning popup, then keep the keyboard open.
    Invalid,
}

/// The warning popup's string ids (FE popup record 0x107).
pub const INVALID_TITLE: &str = "ID_COMMON_WARNING_TITLE";
pub const INVALID_TEXT: &str = "ID_VIRTUALKEYBOARD_ALPHA_NUMERIC_STRING_ERROR";
pub const INVALID_BUTTON: &str = "ID_COMMON_OK";

/// 0x82639890 on a finished entry. `allowed` is the FELanguage `ALL_VALID_ALPHANUMERIC_CHARACTERS` text.
pub fn finish(request: &TextEntryRequest, text: &str, allowed: &str) -> EntryResult {
    let text: String = text.chars().take(request.max_len).collect();
    if text.is_empty() && !request.allow_empty {
        return EntryResult::Reopen;
    }
    if request.mode <= 1 && !text.chars().all(|c| allowed.contains(c)) {
        return EntryResult::Invalid;
    }
    EntryResult::Accepted(text)
}

/// One key of the on-screen keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Space,
    Backspace,
    Shift,
    Done,
    Cancel,
}

/// The grid, rows top to bottom (ours). Letters show lower case; Shift switches to upper case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardLayout {
    pub rows: Vec<Vec<Key>>,
}

impl Default for KeyboardLayout {
    fn default() -> Self {
        let chars = |s: &str| s.chars().map(Key::Char).collect::<Vec<_>>();
        Self {
            rows: vec![
                chars("1234567890"),
                chars("qwertyuiop"),
                chars("asdfghjkl'"),
                chars("zxcvbnm,.?"),
                chars("!@#$*()/_-"),
                vec![Key::Shift, Key::Space, Key::Backspace, Key::Cancel, Key::Done],
            ],
        }
    }
}

/// What the keyboard did with an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyboardOutcome {
    None,
    /// The player finished: run [`finish`] on the text.
    Done(String),
    Cancel,
}

/// Directions and buttons the keyboard reads (pad and arrow keys; typing goes through [`Keyboard::type_char`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardInput {
    Up,
    Down,
    Left,
    Right,
    /// A / Enter on the grid: press the highlighted key.
    Press,
    Backspace,
    Shift,
    /// Start: done from anywhere.
    Done,
    /// B: cancel.
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyboard {
    pub request: TextEntryRequest,
    pub text: String,
    pub row: usize,
    pub col: usize,
    pub shift: bool,
}

impl Keyboard {
    pub fn open(request: TextEntryRequest) -> Self {
        let text = request.default.chars().take(request.max_len).collect();
        Self { request, text, row: 1, col: 0, shift: false }
    }

    /// The key under the highlight.
    pub fn key(&self, layout: &KeyboardLayout) -> Option<Key> {
        layout.rows.get(self.row).and_then(|r| r.get(self.col.min(r.len().saturating_sub(1)))).copied()
    }

    /// A key's character as it would be typed now (upper case while Shift is on).
    pub fn shown(&self, key: Key) -> Option<char> {
        match key {
            Key::Char(c) if self.shift => c.to_uppercase().next(),
            Key::Char(c) => Some(c),
            Key::Space => Some(' '),
            _ => None,
        }
    }

    /// Typed text from a PC keyboard (one character).
    pub fn type_char(&mut self, c: char) {
        if !c.is_control() && self.text.chars().count() < self.request.max_len {
            self.text.push(c);
        }
    }

    pub fn input(&mut self, input: KeyboardInput, layout: &KeyboardLayout) -> KeyboardOutcome {
        let rows = layout.rows.len().max(1);
        match input {
            KeyboardInput::Up => self.row = (self.row + rows - 1) % rows,
            KeyboardInput::Down => self.row = (self.row + 1) % rows,
            KeyboardInput::Left | KeyboardInput::Right => {
                let len = layout.rows.get(self.row).map_or(1, |r| r.len().max(1));
                let col = self.col.min(len - 1);
                self.col = if input == KeyboardInput::Left { (col + len - 1) % len } else { (col + 1) % len };
            }
            KeyboardInput::Backspace => {
                self.text.pop();
            }
            KeyboardInput::Shift => self.shift = !self.shift,
            KeyboardInput::Done => return KeyboardOutcome::Done(self.text.clone()),
            KeyboardInput::Cancel => return KeyboardOutcome::Cancel,
            KeyboardInput::Press => match self.key(layout) {
                Some(k @ (Key::Char(_) | Key::Space)) => {
                    if let Some(c) = self.shown(k) {
                        self.type_char(c);
                    }
                }
                Some(Key::Backspace) => {
                    self.text.pop();
                }
                Some(Key::Shift) => self.shift = !self.shift,
                Some(Key::Done) => return KeyboardOutcome::Done(self.text.clone()),
                Some(Key::Cancel) => return KeyboardOutcome::Cancel,
                None => {}
            },
        }
        KeyboardOutcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOWED: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789',.?!@#$*()\"/_ -,.' ";

    #[test]
    fn the_ok_path_keeps_case_checks_characters_and_the_empty_rule() {
        let cheat = TextEntryRequest::cheat_code("ID_EXTRA_CHEAT_CODE");
        assert_eq!(finish(&cheat, "Zombie", ALLOWED), EntryResult::Accepted("Zombie".into()), "no case change");
        assert_eq!(finish(&cheat, "", ALLOWED), EntryResult::Accepted(String::new()), "the cheat row allows empty");
        assert_eq!(finish(&cheat, "zom~bie", ALLOWED), EntryResult::Invalid);
        let strict = TextEntryRequest { allow_empty: false, ..cheat.clone() };
        assert_eq!(finish(&strict, "", ALLOWED), EntryResult::Reopen);
        let any = TextEntryRequest { mode: 2, ..cheat.clone() };
        assert_eq!(finish(&any, "zom~bie", ALLOWED), EntryResult::Accepted("zom~bie".into()), "modes 2+ skip the check");
        let short = TextEntryRequest { max_len: 3, ..cheat };
        assert_eq!(finish(&short, "zombie", ALLOWED), EntryResult::Accepted("zom".into()));
    }

    #[test]
    fn the_grid_types_a_code_with_the_pad() {
        let layout = KeyboardLayout::default();
        let mut k = Keyboard::open(TextEntryRequest::cheat_code("t"));
        // Row 1 starts at q; "zombie": z is row 3 col 0.
        let press_at = |k: &mut Keyboard, row: usize, col: usize| {
            while k.row != row {
                k.input(KeyboardInput::Down, &layout);
            }
            while k.col != col {
                k.input(KeyboardInput::Right, &layout);
            }
            k.input(KeyboardInput::Press, &layout)
        };
        for (r, c) in [(3, 0), (1, 8), (3, 6), (3, 4), (1, 7), (1, 2)] {
            press_at(&mut k, r, c);
        }
        assert_eq!(k.text, "zombie");
        k.input(KeyboardInput::Shift, &layout);
        press_at(&mut k, 2, 0);
        assert_eq!(k.text, "zombieA", "shift types upper case");
        k.input(KeyboardInput::Backspace, &layout);
        assert_eq!(k.input(KeyboardInput::Done, &layout), KeyboardOutcome::Done("zombie".into()));
        assert_eq!(k.input(KeyboardInput::Cancel, &layout), KeyboardOutcome::Cancel);
    }

    #[test]
    fn moving_wraps_and_short_rows_clamp_the_column() {
        let layout = KeyboardLayout::default();
        let mut k = Keyboard::open(TextEntryRequest::cheat_code("t"));
        k.input(KeyboardInput::Up, &layout);
        k.input(KeyboardInput::Up, &layout);
        assert_eq!(k.row, layout.rows.len() - 1, "up from row 0 wraps to the bottom");
        k.col = 9;
        assert_eq!(k.key(&layout), Some(Key::Done), "the bottom row is shorter: the last key");
        k.input(KeyboardInput::Right, &layout);
        assert_eq!(k.col, 0);
    }

    #[test]
    fn typing_stops_at_the_limit() {
        let mut k = Keyboard::open(TextEntryRequest { max_len: 2, ..TextEntryRequest::cheat_code("t") });
        for c in "abc".chars() {
            k.type_char(c);
        }
        assert_eq!(k.text, "ab");
    }
}
