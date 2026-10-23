//! The text viewer (doc 31 "Extras: Licenses"): retail FE screen 3, ToSScreen (vtable 0x82304F10, template 28
//! `nucleus/nucleus_tos.swf`, research b322), the Nucleus terms-of-service screen the Extras Licenses row reuses in its
//! view-only state. The C++ splits the body into words at its delimiters (0x8263AD78: space, LF, CR, TAB; word i runs
//! from boundary i to boundary i + 1, so every word after the first keeps its leading delimiter and the movie's layout
//! sees the line breaks) up to a character cap (setup flag +0x31: 81626); the movie asks for them through the `tos`
//! object (GetHeaderText, GetTOSstate, GetNumWords, GetWordRange) and lays them out itself, 20 lines, scrolled by the
//! left stick. Back plays core_b_button and closes the screen in view mode.

/// GetTOSstate: view only (the licenses) vs account creation (an Accept button).
pub const STATE_VIEW: i32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerRules {
    /// Characters split into words (later text is not shown): retail 81626 (99167 for one language).
    pub max_chars: usize,
    pub sound_back: String,
}

impl Default for ViewerRules {
    fn default() -> Self {
        Self { max_chars: 81626, sound_back: "core_b_button".into() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextViewer {
    /// The header (string id or `#literal text`).
    pub title: String,
    pub words: Vec<String>,
    pub state: i32,
}

impl TextViewer {
    /// Setup `sub_8263A6D8`: title and body, split into words.
    pub fn open(title: &str, body: &str, rules: &ViewerRules) -> Self {
        let mut words: Vec<String> = Vec::new();
        for c in body.chars().take(rules.max_chars) {
            if matches!(c, ' ' | '\n' | '\r' | '\t') || words.is_empty() {
                words.push(String::new());
            }
            if let Some(w) = words.last_mut() {
                w.push(c);
            }
        }
        Self { title: title.into(), words, state: STATE_VIEW }
    }

    /// GetWordRange(start, count): the words from `start`, at most `count`.
    pub fn range(&self, start: usize, count: usize) -> &[String] {
        let start = start.min(self.words.len());
        &self.words[start..(start + count).min(self.words.len())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_keep_their_leading_delimiter() {
        let v = TextViewer::open("ID_EXTRA_LICENSES_TITLE", "a b\tc\nd", &ViewerRules::default());
        assert_eq!(v.words, ["a", " b", "\tc", "\nd"]);
        assert_eq!(v.words.concat(), "a b\tc\nd", "the movie rebuilds the text by concatenating");
        assert_eq!(v.state, STATE_VIEW);
        assert_eq!(v.range(2, 5), ["\tc", "\nd"]);
        assert!(v.range(10, 5).is_empty());
    }

    #[test]
    fn text_past_the_cap_is_not_split() {
        let v = TextViewer::open("t", "aaa bbb ccc", &ViewerRules { max_chars: 7, ..Default::default() });
        assert_eq!(v.words, ["aaa", " bbb"]);
    }
}
