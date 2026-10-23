//! Game Settings > Video > Video Calibration (doc 31 "Video Calibration"): retail FE screen 51 (vtable 0x82303548,
//! template 91 VideoCalibration, `options/videocal`). A guide for setting up the TV in pages (settings, contrast,
//! brightness); it changes no game value and saves nothing. The screen keeps one counter, the next page to show
//! (screen +60):
//! - init 0x8260E868: counter 0; loaded 0x8260E888 and A run the advance 0x8260E950: below the page count it posts
//!   SetState(counter) and counts up, else it asks the "finished" popup (title / description / Yes / No);
//! - B (input 0x8260E890): below 2 the screen closes, else SetState(counter - 2) and counts down;
//! - popup callback 0x8260EA48: choice 0 closes, any other starts over at page 0.
//!
//! Values are data (`CalibrationRules`), retail's the defaults, so a mod can add or drop pages.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalibrationRules {
    /// Pages before the popup (0x8260E950 compares with 3): settings 0, contrast 1, brightness 2.
    pub pages: u32,
    /// The popup's string ids (0x8260E950: title, description, the two buttons; choice 0 = the first).
    pub popup_title: String,
    pub popup_description: String,
    pub popup_buttons: Vec<String>,
    /// UI sounds by fe record name: A (h64 0x373D38D7EA167A71) and B (h64 0x4D927CD64CC8B818).
    pub sound_next: String,
    pub sound_back: String,
}

impl Default for CalibrationRules {
    fn default() -> Self {
        Self {
            pages: 3,
            popup_title: "ID_VIDEO_CALIBRATION_FINISHED_TITLE".into(),
            popup_description: "ID_VIDEO_CALIBRATION_FINISHED_DESC".into(),
            popup_buttons: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
            sound_next: "core_a_button".into(),
            sound_back: "core_b_button".into(),
        }
    }
}

/// What the host does after an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationAction {
    /// Post AS SetState(page) to the movie.
    SetState(u32),
    /// Close (pop) the screen.
    Close,
    /// Ask the "finished" popup; answer with [`VideoCalibration::popup_result`].
    Popup,
}

/// The screen's counter (+60): the next page to show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoCalibration {
    pub next: u32,
}

impl VideoCalibration {
    /// Init 0x8260E868.
    pub fn open() -> Self {
        Self { next: 0 }
    }

    /// 0x8260E950 (loaded, and A).
    pub fn advance(&mut self, rules: &CalibrationRules) -> CalibrationAction {
        if self.next < rules.pages {
            self.next += 1;
            CalibrationAction::SetState(self.next - 1)
        } else {
            CalibrationAction::Popup
        }
    }

    /// B in 0x8260E890.
    pub fn back(&mut self) -> CalibrationAction {
        if self.next < 2 {
            CalibrationAction::Close
        } else {
            self.next -= 1;
            CalibrationAction::SetState(self.next - 1)
        }
    }

    /// Popup callback 0x8260EA48: choice 0 closes, any other starts over.
    pub fn popup_result(&mut self, choice: u32, rules: &CalibrationRules) -> CalibrationAction {
        if choice == 0 {
            CalibrationAction::Close
        } else {
            self.next = 0;
            self.advance(rules)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CalibrationAction::*;

    #[test]
    fn pages_step_like_retail() {
        let rules = CalibrationRules::default();
        let mut c = VideoCalibration::open();
        // Loaded shows page 0; A walks to 1 and 2, then the popup (the counter stays at 3).
        assert_eq!(c.advance(&rules), SetState(0));
        assert_eq!(c.advance(&rules), SetState(1));
        assert_eq!(c.advance(&rules), SetState(2));
        assert_eq!(c.advance(&rules), Popup);
        assert_eq!(c.next, 3);
        // B from the last page goes back one page at a time, and closes from page 0.
        assert_eq!(c.back(), SetState(1));
        assert_eq!(c.back(), SetState(0));
        assert_eq!(c.back(), Close);
    }

    #[test]
    fn popup_yes_closes_no_starts_over() {
        let rules = CalibrationRules::default();
        let mut c = VideoCalibration { next: 3 };
        assert_eq!(c.popup_result(1, &rules), SetState(0));
        assert_eq!(c.next, 1);
        let mut c = VideoCalibration { next: 3 };
        assert_eq!(c.popup_result(0, &rules), Close);
    }

    #[test]
    fn page_count_is_data() {
        let rules = CalibrationRules { pages: 1, ..Default::default() };
        let mut c = VideoCalibration::open();
        assert_eq!(c.advance(&rules), SetState(0));
        assert_eq!(c.advance(&rules), Popup);
    }
}
