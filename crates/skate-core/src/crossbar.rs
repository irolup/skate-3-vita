//! Pause menu ("crossbar") navigation, ported from the retail C++ screen (doc 31, Milestone 6).
//!
//! Retail keeps all navigation state in C++ (crossbar screen fields +368 tab, +372 row, +376 sub-option, +384 level);
//! the core_menu movie only mirrors it through the CoreMenu getters and redraws when the C++ posts `SetState(n)`.
//! The handler is sub_8261B560 (TU3): per input event it moves the indices, plays an fe sound and posts a state.
//! This module is that handler over a `CrossbarView` (the current mode's tabs and rows, from the menu registry), so
//! the rows, their enabled rules and the sounds / state numbers stay data that mods can change.
//!
//! Engine-independent: no rendering, I/O or ECS.

/// The menu content the handler reads (retail: the mode object at screen+480 and IsItemEnabled).
pub trait CrossbarView {
    /// Tab count (mode vt+16).
    fn tab_count(&self) -> usize;
    /// Visible rows of a tab (mode vt+20).
    fn row_count(&self, tab: usize) -> usize;
    /// IsItemEnabled (sub_8261E208).
    fn row_enabled(&self, tab: usize, row: usize) -> bool;
    /// The row's sub-option kind (item table +12), -1 = none.
    fn sub_option_kind(&self, tab: usize, row: usize) -> i32;
    /// GetNumSubOptions (sub_8261DB48).
    fn sub_option_count(&self, tab: usize, row: usize) -> usize;
    /// The mode's X-button hook (mode vt+28) answers 1, e.g. the DLC offer is available.
    fn extra_available(&self) -> bool {
        false
    }
}

/// Input events, numbered like retail's Apt key index (key table 0x83027BF0), which the handler receives as is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavEvent {
    /// 2 AptNext (A).
    Select,
    /// 3 AptPrevious (B).
    Back,
    /// 5 AptMisc (X).
    Extra,
    /// 12 AptUp.
    Up,
    /// 13 AptDown.
    Down,
    /// 14 AptLeft.
    TabLeft,
    /// 15 AptRight.
    TabRight,
}

impl NavEvent {
    pub fn from_key_index(index: u32) -> Option<Self> {
        Some(match index {
            2 => Self::Select,
            3 => Self::Back,
            5 => Self::Extra,
            12 => Self::Up,
            13 => Self::Down,
            14 => Self::TabLeft,
            15 => Self::TabRight,
            _ => return None,
        })
    }
}

/// Sounds and SetState numbers; retail values by default (sub_8261B560, sub_8261B9C8), overridable as data.
#[derive(Clone, Debug, PartialEq)]
pub struct CrossbarRules {
    /// fe records played per event (keys decoded from sub_8261B560, names via the fe vault hash).
    pub sound_up: String,
    pub sound_down: String,
    pub sound_left: String,
    pub sound_right: String,
    pub sound_select: String,
    pub sound_back: String,
    pub sound_extra: String,
    /// FECrossBarScreenMode_* values (= their index in the table registered by sub_8261B9C8).
    pub state_refresh_menu: i32,
    pub state_refresh_option: i32,
    pub state_initialise_menu: i32,
    pub state_initialise_sub_option: i32,
    pub state_init_sub_option_online_team: i32,
    pub state_refresh_dlc_banner: i32,
    /// Sub-option kind of the online team item (kind 5): retail may hold it pending instead of opening.
    pub online_team_kind: i32,
}

impl Default for CrossbarRules {
    fn default() -> Self {
        Self {
            sound_up: "crossbar_up".into(),
            sound_down: "crossbar_down".into(),
            sound_left: "crossbar_left".into(),
            sound_right: "crossbar_right".into(),
            sound_select: "core_a_button".into(),
            sound_back: "core_b_button".into(),
            sound_extra: "core_x_button".into(),
            state_refresh_menu: 0,
            state_refresh_option: 1,
            state_initialise_menu: 2,
            state_initialise_sub_option: 3,
            state_init_sub_option_online_team: 4,
            state_refresh_dlc_banner: 5,
            online_team_kind: 5,
        }
    }
}

/// OPTION_MENULEVEL / SUBOPTION_MENULEVEL (core_menu constants, screen+384).
pub const OPTION_LEVEL: i32 = 1;
pub const SUB_OPTION_LEVEL: i32 = 2;

/// The crossbar screen's navigation fields. Serialisable plain data (multiplayer, replays).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrossbarState {
    /// +368
    pub menu: i32,
    /// +372 (-1 = none)
    pub option: i32,
    /// +376
    pub sub: i32,
    /// +384
    pub level: i32,
    /// +389: an X action is being started.
    pub extra_busy: bool,
    /// +390: the DLC banner / X action is offered.
    pub dlc: bool,
    /// +392: the online team sub-option waits for data; blocks input.
    pub online_team_pending: bool,
    /// +400
    pub dlc_banner: i32,
}

/// What the host does after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossbarAction {
    /// The sub-option column opened for this kind (the movie draws it after SetState).
    OpenSubColumn { kind: i32 },
    /// A row without sub-options was selected: the mode's select hook, then the item's own action
    /// (retail sub_8261BC30 steps 3 and 4; ours: the registry handlers / interceptors).
    SelectItem { tab: usize, row: usize },
    /// skate_vita: A on a sub-option row (the item's column): the host runs the item's choice. NOT RETAIL YET: the
    /// retail sub-option select cases (sub_8261BC30 by kind) are not decoded.
    SelectSubOption { tab: usize, row: usize, sub: usize },
    /// Back at the option level: close the pause menu (retail sub_8261AFA0, close of FE id 56).
    Close,
    /// The X action (retail opens FE screen 19 when sub_8261CE38 agrees).
    Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavOutcome {
    pub sound: Option<String>,
    /// `SetState(n)` to post to the movie.
    pub set_state: Option<i32>,
    pub action: Option<CrossbarAction>,
    /// The handler consumed the key (retail returns 1 only for back at level 1); otherwise the key also goes to
    /// the movie's UpdateInput.
    pub consumed: bool,
}

impl CrossbarState {
    /// Screen init: level 1, first enabled row of the first tab.
    pub fn open(view: &dyn CrossbarView) -> Self {
        let mut state = Self { level: OPTION_LEVEL, ..Self::default() };
        state.first_enabled_row(view, true);
        state
    }

    /// sub_8261F0E8: clamp a missing / out of range row to 0, then walk rows (down: +1, up: -1) with wrap until an
    /// enabled one; after two wraps without one the row is 0 and the result false.
    pub fn first_enabled_row(&mut self, view: &dyn CrossbarView, down: bool) -> bool {
        let tab = self.menu.max(0) as usize;
        let count = view.row_count(tab) as i32;
        if self.option == -1 || self.option >= count {
            self.option = 0;
        }
        let mut wraps = 0;
        loop {
            if self.option >= 0 && self.option < count && view.row_enabled(tab, self.option as usize) {
                return true;
            }
            if down {
                self.option += 1;
                if self.option < count {
                    continue;
                }
                wraps += 1;
                self.option = 0;
            } else {
                self.option -= 1;
                if self.option >= 0 {
                    continue;
                }
                wraps += 1;
                self.option = count - 1;
            }
            if wraps >= 2 {
                self.option = 0;
                return false;
            }
        }
    }

    /// sub_8261B560 for one event. `blocked` is retail's global gate (FE input block / busy flag).
    pub fn handle(&mut self, event: NavEvent, view: &dyn CrossbarView, rules: &CrossbarRules, blocked: bool) -> NavOutcome {
        let mut out = NavOutcome::default();
        if blocked || self.online_team_pending {
            return out;
        }
        if event == NavEvent::Extra {
            if self.dlc && view.extra_available() && !self.extra_busy {
                self.extra_busy = true;
                out.sound = Some(rules.sound_extra.clone());
                out.action = Some(CrossbarAction::Extra);
            }
            return out;
        }
        let tab = self.menu.max(0) as usize;
        if self.level == SUB_OPTION_LEVEL {
            let row = self.option.max(0) as usize;
            match event {
                NavEvent::Up | NavEvent::Down => {
                    let count = view.sub_option_count(tab, row) as i32;
                    if count > 0 {
                        self.sub += if event == NavEvent::Up { -1 } else { 1 };
                        if self.sub < 0 {
                            self.sub = count - 1;
                        } else if self.sub > count - 1 {
                            self.sub = 0;
                        }
                    }
                }
                NavEvent::Back => {
                    out.sound = Some(rules.sound_back.clone());
                    self.level = OPTION_LEVEL;
                }
                NavEvent::Select if view.sub_option_count(tab, row) > 0 => {
                    out.sound = Some(rules.sound_select.clone());
                    out.action = Some(CrossbarAction::SelectSubOption { tab, row, sub: self.sub.max(0) as usize });
                }
                _ => {}
            }
            return out;
        }
        match event {
            NavEvent::Up | NavEvent::Down => {
                let up = event == NavEvent::Up;
                let count = view.row_count(tab) as i32;
                self.option += if up { -1 } else { 1 };
                if self.option < 0 {
                    self.option = count - 1;
                } else if self.option > count - 1 {
                    self.option = 0;
                }
                let found = self.first_enabled_row(view, !up);
                self.level = OPTION_LEVEL;
                if found {
                    out.sound = Some(if up { rules.sound_up.clone() } else { rules.sound_down.clone() });
                    out.set_state = Some(rules.state_refresh_option);
                }
            }
            NavEvent::TabLeft | NavEvent::TabRight => {
                let left = event == NavEvent::TabLeft;
                out.sound = Some(if left { rules.sound_left.clone() } else { rules.sound_right.clone() });
                let count = view.tab_count() as i32;
                self.menu += if left { -1 } else { 1 };
                if self.menu < 0 {
                    self.menu = count - 1;
                } else if self.menu > count - 1 {
                    self.menu = 0;
                }
                // sub_8261D9F8; the sub-option index is kept.
                self.level = OPTION_LEVEL;
                self.option = 0;
                self.first_enabled_row(view, true);
                out.set_state = Some(rules.state_refresh_menu);
            }
            NavEvent::Select => {
                if self.option < 0 || !view.row_enabled(tab, self.option as usize) {
                    return out;
                }
                out.sound = Some(rules.sound_select.clone());
                self.extra_busy = false;
                let row = self.option as usize;
                let kind = view.sub_option_kind(tab, row);
                if kind != -1 {
                    // sub_8261DA58. NOT PORTED: the online team kind's pending path (online only).
                    self.online_team_pending = false;
                    self.level = SUB_OPTION_LEVEL;
                    self.sub = 0;
                    out.set_state = Some(rules.state_initialise_sub_option);
                    out.action = Some(CrossbarAction::OpenSubColumn { kind });
                } else {
                    self.online_team_pending = false;
                    out.action = Some(CrossbarAction::SelectItem { tab, row });
                }
            }
            NavEvent::Back => {
                out.sound = Some(rules.sound_back.clone());
                out.action = Some(CrossbarAction::Close);
                out.consumed = true;
            }
            NavEvent::Extra => {}
        }
        out
    }
}

/// The current mode's tabs and rows from the menu registry (visible entries only, enabled by its rules and
/// handlers). Sub-option counts per kind come from the sub-option tables (retail 0x83027958, 6 wide).
pub struct RegistryView {
    pub tabs: Vec<String>,
    pub rows: Vec<Vec<crate::menus::Row>>,
    pub kinds: Vec<Vec<i32>>,
    /// Sub-option row count per kind.
    pub sub_options: std::collections::BTreeMap<i32, usize>,
    pub extra: bool,
}

impl RegistryView {
    pub fn new(
        registry: &crate::menus::MenuRegistry,
        ctx: &crate::menus::MenuContext,
        sub_options: std::collections::BTreeMap<i32, usize>,
    ) -> Self {
        let tabs = registry.tabs(ctx);
        let rows: Vec<Vec<crate::menus::Row>> = tabs.iter().map(|t| registry.tab_rows(t, ctx)).collect();
        let kinds = rows
            .iter()
            .map(|r| r.iter().map(|row| registry.data().item(&row.id).map_or(-1, |i| i.sub_option_kind)).collect())
            .collect();
        Self { tabs, rows, kinds, sub_options, extra: false }
    }
    /// Item id at a position (stable id for selection events).
    pub fn item(&self, tab: usize, row: usize) -> Option<&str> {
        self.rows.get(tab)?.get(row).map(|r| r.id.as_str())
    }
}

impl CrossbarView for RegistryView {
    fn tab_count(&self) -> usize {
        self.tabs.len()
    }
    fn row_count(&self, tab: usize) -> usize {
        self.rows.get(tab).map_or(0, Vec::len)
    }
    fn row_enabled(&self, tab: usize, row: usize) -> bool {
        self.rows.get(tab).and_then(|r| r.get(row)).is_some_and(|r| r.enabled)
    }
    fn sub_option_kind(&self, tab: usize, row: usize) -> i32 {
        self.kinds.get(tab).and_then(|k| k.get(row)).copied().unwrap_or(-1)
    }
    fn sub_option_count(&self, tab: usize, row: usize) -> usize {
        self.sub_options.get(&self.sub_option_kind(tab, row)).copied().unwrap_or(0)
    }
    fn extra_available(&self) -> bool {
        self.extra
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tabs of rows; `false` = disabled; kind per row.
    struct View(Vec<Vec<(bool, i32)>>);
    impl CrossbarView for View {
        fn tab_count(&self) -> usize {
            self.0.len()
        }
        fn row_count(&self, tab: usize) -> usize {
            self.0[tab].len()
        }
        fn row_enabled(&self, tab: usize, row: usize) -> bool {
            self.0[tab][row].0
        }
        fn sub_option_kind(&self, tab: usize, row: usize) -> i32 {
            self.0[tab][row].1
        }
        fn sub_option_count(&self, _: usize, _: usize) -> usize {
            3
        }
    }

    fn view() -> View {
        View(vec![
            vec![(true, -1), (false, -1), (true, -1), (true, 0)],
            vec![(false, -1), (true, -1)],
            vec![(false, -1), (false, -1)],
        ])
    }

    #[test]
    fn crossbar_rows_skip_disabled_and_wrap_like_retail() {
        let v = view();
        let r = CrossbarRules::default();
        let mut s = CrossbarState::open(&v);
        assert_eq!((s.menu, s.option, s.level), (0, 0, 1));
        let o = s.handle(NavEvent::Down, &v, &r, false);
        assert_eq!(s.option, 2, "row 1 is disabled");
        assert_eq!(o.sound.as_deref(), Some("crossbar_down"));
        assert_eq!(o.set_state, Some(1));
        s.handle(NavEvent::Down, &v, &r, false);
        s.handle(NavEvent::Down, &v, &r, false);
        assert_eq!(s.option, 0, "wraps to the top");
        let o = s.handle(NavEvent::Up, &v, &r, false);
        assert_eq!((s.option, o.sound.as_deref()), (3, Some("crossbar_up")));
        s.handle(NavEvent::Up, &v, &r, false);
        assert_eq!(s.option, 2);
        s.handle(NavEvent::Up, &v, &r, false);
        assert_eq!(s.option, 0, "up skips row 1 downwards in index");
    }

    #[test]
    fn crossbar_tabs_reset_the_row_and_post_refresh_menu() {
        let v = view();
        let r = CrossbarRules::default();
        let mut s = CrossbarState::open(&v);
        let o = s.handle(NavEvent::TabRight, &v, &r, false);
        assert_eq!((s.menu, s.option), (1, 1));
        assert_eq!((o.sound.as_deref(), o.set_state), (Some("crossbar_right"), Some(0)));
        // A tab with no enabled row: row 0, still the sound and SetState 0 (retail D9F8 ignores the result).
        let o = s.handle(NavEvent::TabRight, &v, &r, false);
        assert_eq!((s.menu, s.option, o.set_state), (2, 0, Some(0)));
        // Up there finds nothing: no sound, no SetState, index already written.
        let o = s.handle(NavEvent::Up, &v, &r, false);
        assert_eq!((o.sound, o.set_state, s.option), (None, None, 0));
        s.handle(NavEvent::TabRight, &v, &r, false);
        assert_eq!(s.menu, 0, "tabs wrap");
        s.handle(NavEvent::TabLeft, &v, &r, false);
        assert_eq!(s.menu, 2);
    }

    #[test]
    fn crossbar_select_opens_sub_column_or_selects_and_back_closes() {
        let v = view();
        let r = CrossbarRules::default();
        let mut s = CrossbarState::open(&v);
        let o = s.handle(NavEvent::Select, &v, &r, false);
        assert_eq!(o.action, Some(CrossbarAction::SelectItem { tab: 0, row: 0 }));
        assert_eq!((o.sound.as_deref(), o.set_state), (Some("core_a_button"), None));
        s.option = 3;
        let o = s.handle(NavEvent::Select, &v, &r, false);
        assert_eq!(o.action, Some(CrossbarAction::OpenSubColumn { kind: 0 }));
        assert_eq!((s.level, s.sub, o.set_state), (2, 0, Some(3)));
        // Sub column: silent wrap, tab keys ignored, back returns to level 1 keeping the sub index.
        let o = s.handle(NavEvent::Up, &v, &r, false);
        assert_eq!((s.sub, o.sound), (2, None));
        assert_eq!(s.handle(NavEvent::TabLeft, &v, &r, false), NavOutcome::default());
        let o = s.handle(NavEvent::Back, &v, &r, false);
        assert_eq!((s.level, s.sub, o.sound.as_deref(), o.consumed), (1, 2, Some("core_b_button"), false));
        let o = s.handle(NavEvent::Back, &v, &r, false);
        assert_eq!((o.action, o.consumed), (Some(CrossbarAction::Close), true));
        // Disabled rows cannot be selected; the gate blocks everything.
        s.option = 1;
        assert_eq!(s.handle(NavEvent::Select, &v, &r, false), NavOutcome::default());
        assert_eq!(s.handle(NavEvent::Down, &v, &r, true), NavOutcome::default());
    }
}
