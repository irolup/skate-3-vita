//! Free Play pause row Skate Profile (doc 31 "Skate Profile"): retail FE screen 61 (vtable 0x82305448, template 106
//! OfflineProfile, `profile/offline_skateProfile`). The career view in three tabs (progression, stats, personal
//! bests); the movie reads its pages from the Progression and Stats host objects. The C++ side keeps one value, the
//! tab index (screen +0x3C, research b308):
//! - init 0x8264C530: tab 0, then the refresh 0x8264C830 (tab -> the tab's SetState);
//! - input 0x8264C608: Back plays core_b_button and pops the screen; the tab keys play core_tab, step the tab
//!   (clamped: at either end it stays), then SetState(30) (the movie syncs its tab highlight) and the refresh, also
//!   at either end; Up / Down play profile_career_scroll and stay unconsumed, so the movie scrolls its list.
//!
//! Values are data (`ProfileRules`), retail's the defaults, so a mod can add, drop or reorder tabs.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTab {
    /// The tab label (FELanguage id; the movie's SetupTabs list).
    pub label: String,
    /// The movie state that shows the tab's page (0x8264C830).
    pub state: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRules {
    pub tabs: Vec<ProfileTab>,
    /// Posted after a tab key, before the refresh (the movie's tab highlight sync).
    pub sync_state: i32,
    /// UI sounds by fe record name: tab keys (h64 0xFD87566338C118E1), Back (0x4D927CD64CC8B818), Up / Down
    /// (0x16469A2827B77C98).
    pub sound_tab: String,
    pub sound_back: String,
    pub sound_scroll: String,
}

impl Default for ProfileRules {
    fn default() -> Self {
        let tab = |label: &str, state| ProfileTab { label: label.into(), state };
        Self {
            tabs: vec![
                tab("ID_PROFILE_SINGLEPLAYER_TAB_PROGRESSION", 5),
                tab("ID_PROFILE_SINGLEPLAYER_TAB_STATS", 10),
                tab("ID_DPO_PERSONALBEST", 20),
            ],
            sync_state: 30,
            sound_tab: "core_tab".into(),
            sound_back: "core_b_button".into(),
            sound_scroll: "profile_career_scroll".into(),
        }
    }
}

/// One overall bar (Progression GetOverallCareerData 0x825B0F20 / GetOverallChallengeData 0x825B0F38): the movie
/// reads [title, icon asset, current, total] and shows "current / total", the bar and the star at full.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileOverall {
    pub title: String,
    pub icon: String,
    pub current: f64,
    pub total: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatValue {
    Number(f64),
    Text(String),
}

/// One Stats / Personal Best table row (GetStatsChunk 0x825ABA28 builds [label, value] per enabled entry); `id` is
/// the stable key mods replace rows by.
#[derive(Clone, Debug, PartialEq)]
pub struct StatRow {
    pub id: String,
    pub label: String,
    pub value: StatValue,
}

/// What the pages show. NOT RETAIL YET: the engine has no career, challenge or stats records, so the defaults are
/// empty (bars at 0 / 0, no rows) until a career system or a mod (`sdk.menus.profile`) fills them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileData {
    pub career: ProfileOverall,
    pub challenges: ProfileOverall,
    pub stats: Vec<StatRow>,
    pub personal_bests: Vec<StatRow>,
}

/// The keys the screen's C++ handler reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileKey {
    Back,
    NextTab,
    PreviousTab,
    Scroll,
}

/// What the host does after an input: play `sound`, post `states` to the movie in order, close when asked; an
/// unconsumed key goes on to the movie's UpdateInput.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileOutcome {
    pub consumed: bool,
    pub close: bool,
    pub sound: Option<String>,
    pub states: Vec<i32>,
}

/// The screen's tab index (+0x3C).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OfflineProfile {
    pub tab: usize,
}

impl OfflineProfile {
    /// Init 0x8264C530: tab 0 and its page.
    pub fn open(rules: &ProfileRules) -> (Self, Vec<i32>) {
        let screen = Self { tab: 0 };
        let states = screen.refresh(rules).into_iter().collect();
        (screen, states)
    }

    /// Refresh 0x8264C830: the current tab's page.
    pub fn refresh(&self, rules: &ProfileRules) -> Option<i32> {
        rules.tabs.get(self.tab).map(|t| t.state)
    }

    /// Input 0x8264C608.
    pub fn key(&mut self, key: ProfileKey, rules: &ProfileRules) -> ProfileOutcome {
        match key {
            ProfileKey::Back => ProfileOutcome { consumed: true, close: true, sound: Some(rules.sound_back.clone()), states: Vec::new() },
            ProfileKey::NextTab | ProfileKey::PreviousTab => {
                let last = rules.tabs.len().saturating_sub(1);
                self.tab = if key == ProfileKey::NextTab { (self.tab + 1).min(last) } else { self.tab.saturating_sub(1) };
                let mut states = vec![rules.sync_state];
                states.extend(self.refresh(rules));
                ProfileOutcome { consumed: true, close: false, sound: Some(rules.sound_tab.clone()), states }
            }
            ProfileKey::Scroll => ProfileOutcome { consumed: false, close: false, sound: Some(rules.sound_scroll.clone()), states: Vec::new() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_on_the_progression_page() {
        let rules = ProfileRules::default();
        let (screen, states) = OfflineProfile::open(&rules);
        assert_eq!(screen.tab, 0);
        assert_eq!(states, [5]);
    }

    #[test]
    fn tab_keys_sync_then_show_the_page_and_stay_at_either_end() {
        let rules = ProfileRules::default();
        let (mut screen, _) = OfflineProfile::open(&rules);
        let out = screen.key(ProfileKey::NextTab, &rules);
        assert_eq!((out.consumed, out.states.as_slice(), out.sound.as_deref()), (true, &[30, 10][..], Some("core_tab")));
        screen.key(ProfileKey::NextTab, &rules);
        // At the last tab retail still plays the sound, syncs and refreshes.
        let out = screen.key(ProfileKey::NextTab, &rules);
        assert_eq!((screen.tab, out.states.as_slice()), (2, &[30, 20][..]));
        screen.key(ProfileKey::PreviousTab, &rules);
        screen.key(ProfileKey::PreviousTab, &rules);
        let out = screen.key(ProfileKey::PreviousTab, &rules);
        assert_eq!((screen.tab, out.states.as_slice()), (0, &[30, 5][..]));
    }

    #[test]
    fn back_closes_and_scroll_is_left_to_the_movie() {
        let rules = ProfileRules::default();
        let (mut screen, _) = OfflineProfile::open(&rules);
        let back = screen.key(ProfileKey::Back, &rules);
        assert!(back.consumed && back.close);
        assert_eq!(back.sound.as_deref(), Some("core_b_button"));
        let scroll = screen.key(ProfileKey::Scroll, &rules);
        assert!(!scroll.consumed && !scroll.close && scroll.states.is_empty());
        assert_eq!(scroll.sound.as_deref(), Some("profile_career_scroll"));
    }

    #[test]
    fn a_mod_tab_list_bounds_the_steps() {
        let rules = ProfileRules { tabs: vec![ProfileTab { label: "#Only".into(), state: 5 }], ..Default::default() };
        let (mut screen, _) = OfflineProfile::open(&rules);
        let out = screen.key(ProfileKey::NextTab, &rules);
        assert_eq!((screen.tab, out.states.as_slice()), (0, &[30, 5][..]));
    }
}
