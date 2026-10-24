//! Free Play pause rows Free Play / Call Skater (retail FE 62 SpawnSkater, template `cas/spawnskater`, b543 / b544);
//! groups, modes and limits are data (`SpawnSkaterRules`).
//! Retail handlers and our groups: docs/hails-additions/31-retail-menus.md#spawnskater-fe-62

/// FE 62 +0x40 (`GetScreenMode`): the crossbar row that opened it picks the mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Call Skater (SkateWith, 100): call up to the companion limit, X dismisses.
    SkateWith,
    /// Free Play (SoloFreeskateOptions, 200): become the picked skater, X opens the Free Play options.
    #[default]
    Freeskate,
    /// Demo kiosk pick (300): A only.
    DemoPublic,
}

impl Mode {
    pub fn code(self) -> i32 {
        match self {
            Mode::SkateWith => 100,
            Mode::Freeskate => 200,
            Mode::DemoPublic => 300,
        }
    }
}

/// Who a row is: a character key, or "your skater" (retail save slot 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skater {
    Character(String),
    Own,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub skater: Skater,
    /// A string id, or `#name` for a raw display name (retail formats non-string names with "#%s", 0x822206EC).
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// Stable id (`you`, `teammates`, `pros`, `ip`, a mod's own).
    pub id: String,
    /// Header string id (group +4).
    pub header: String,
    /// The modes the group is listed in (retail's init mask and per-mode predicates, b544).
    pub modes: Vec<Mode>,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnSkaterRules {
    /// Companion slots (`[[0x830CFD94]+0xE0]+0x5C`, 3).
    pub companion_limit: usize,
    /// The crossbar rows that open FE 62 and their mode (b307: 28 -> 200, 37 -> 100).
    pub items: Vec<(String, Mode)>,
    pub sound_call: String,
    pub sound_back: String,
    pub sound_x: String,
    /// The limit popup (ID_CALL_SKATER_LIMIT_REACHED, title ID_CROSSBAR_SKATE_WITH, button ID_COMMON_CONTINUE).
    pub limit_popup: [String; 3],
    /// Movie states (SetState): refresh the list / the button help.
    pub state_list: i32,
    pub state_help: i32,
}

impl Default for SpawnSkaterRules {
    fn default() -> Self {
        Self {
            companion_limit: 3,
            items: vec![("SoloFreeskateOptions".into(), Mode::Freeskate), ("SkateWith".into(), Mode::SkateWith)],
            // fe record keys 0xF26173CBBC2075B4 / 0x4D927CD64CC8B818 / 0x17472B6D55E36C95 (b543); the first and last
            // are not named yet, so the select and tab sounds stand in. NOT RETAIL YET.
            sound_call: "core_a_button".into(),
            sound_back: "core_b_button".into(),
            sound_x: "core_a_button".into(),
            limit_popup: ["ID_CALL_SKATER_LIMIT_REACHED".into(), "ID_CROSSBAR_SKATE_WITH".into(), "ID_COMMON_CONTINUE".into()],
            state_list: 1,
            state_help: 2,
        }
    }
}

/// What a key or pick asks of the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Call a companion (group vt+0x10 -> 827C5C50).
    Call(Skater),
    /// Dismiss a called companion (vt+0x18).
    Dismiss(Skater),
    /// Become this skater (vt+0x1C, then FlowMessage 0x89).
    SetPlayer(Skater),
    /// All companion slots in use: the limit popup.
    LimitReached,
    /// Free Play options (FlowMessage 0x8A, FE 34).
    Options,
    /// Show the highlighted skater (FlowMessage 0x8D).
    Preview(Skater),
    /// Close the screen.
    Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub actions: Vec<Action>,
    pub sound: Option<String>,
    pub states: Vec<i32>,
}

/// One list row (`GetSkatersData`: name, isHeader, spawned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub header: bool,
    pub spawned: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnSkater {
    pub mode: Mode,
    /// The groups shown in this mode (empty groups dropped), in retail order.
    pub groups: Vec<Group>,
    /// Called companions, oldest first.
    pub companions: Vec<Skater>,
    /// FE 62 +0x48 (`GetLastMenuHighlightIndex`, -1 none).
    pub highlight: Option<usize>,
}

impl SpawnSkater {
    /// Init 0x8264D958: the mode's groups, highlight none.
    pub fn open(mode: Mode, groups: &[Group], companions: Vec<Skater>) -> Self {
        let groups = groups.iter().filter(|g| g.modes.contains(&mode) && !g.entries.is_empty()).cloned().collect();
        Self { mode, groups, companions, highlight: None }
    }

    /// sub_8264E2F8: one header per group plus its entries.
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        for g in &self.groups {
            out.push(Row { label: g.header.clone(), header: true, spawned: false });
            out.extend(g.entries.iter().map(|e| Row { label: e.label.clone(), header: false, spawned: self.companions.contains(&e.skater) }));
        }
        out
    }

    /// sub_8264E498: the entry at a list row (None on a header).
    pub fn entry(&self, row: usize) -> Option<&Entry> {
        let mut i = row;
        for g in &self.groups {
            if i == 0 {
                return None;
            }
            i -= 1;
            if i < g.entries.len() {
                return Some(&g.entries[i]);
            }
            i -= g.entries.len();
        }
        None
    }

    /// vt+64 0x8264DB30 (A).
    pub fn select(&mut self, rules: &SpawnSkaterRules, row: usize) -> Outcome {
        let Some(skater) = self.entry(row).map(|e| e.skater.clone()) else { return Outcome::default() };
        match self.mode {
            Mode::SkateWith if self.companions.len() >= rules.companion_limit => Outcome { actions: vec![Action::LimitReached], ..Default::default() },
            Mode::SkateWith => {
                if !self.companions.contains(&skater) {
                    self.companions.push(skater.clone());
                }
                Outcome { actions: vec![Action::Call(skater)], sound: Some(rules.sound_call.clone()), states: vec![rules.state_list] }
            }
            Mode::Freeskate | Mode::DemoPublic => Outcome { actions: vec![Action::SetPlayer(skater)], ..Default::default() },
        }
    }

    /// vt+68 0x8264DD68: store the row; Call Skater refreshes the button help, the others preview the skater.
    pub fn hilight(&mut self, rules: &SpawnSkaterRules, row: usize) -> Outcome {
        self.highlight = Some(row);
        let mut out = Outcome { states: vec![rules.state_help], ..Default::default() };
        if self.mode != Mode::SkateWith {
            if let Some(e) = self.entry(row) {
                out.actions.push(Action::Preview(e.skater.clone()));
            }
        }
        out
    }

    /// vt+40 0x8264DF18: B closes (not in the demo), X dismisses (Call Skater, on a called row) or opens the options.
    pub fn back(&self, rules: &SpawnSkaterRules) -> Outcome {
        if self.mode == Mode::DemoPublic {
            return Outcome::default();
        }
        Outcome { actions: vec![Action::Close], sound: Some(rules.sound_back.clone()), states: Vec::new() }
    }

    pub fn x(&mut self, rules: &SpawnSkaterRules) -> Outcome {
        match self.mode {
            Mode::Freeskate => Outcome { actions: vec![Action::Options], sound: Some(rules.sound_x.clone()), states: Vec::new() },
            Mode::SkateWith => {
                let Some(skater) = self.highlight.and_then(|r| self.entry(r)).map(|e| e.skater.clone()) else { return Outcome::default() };
                let Some(i) = self.companions.iter().position(|c| *c == skater) else { return Outcome::default() };
                self.companions.remove(i);
                Outcome { actions: vec![Action::Dismiss(skater)], sound: Some(rules.sound_x.clone()), states: vec![rules.state_list] }
            }
            Mode::DemoPublic => Outcome::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> Vec<Group> {
        let e = |k: &str| Entry { skater: Skater::Character(k.into()), label: format!("#{k}") };
        vec![
            Group { id: "you".into(), header: "ID_FREESKATE_YOU_HEADER".into(), modes: vec![Mode::Freeskate], entries: vec![Entry { skater: Skater::Own, label: "#You".into() }] },
            Group { id: "pros".into(), header: "ID_CIC_PRO_SKATERS".into(), modes: vec![Mode::SkateWith, Mode::Freeskate], entries: vec![e("a"), e("b"), e("c"), e("d")] },
            Group { id: "ip".into(), header: "ID_CIC_IP_SKATERS".into(), modes: vec![Mode::Freeskate], entries: Vec::new() },
        ]
    }

    #[test]
    fn rows_have_one_header_per_listed_group() {
        let s = SpawnSkater::open(Mode::SkateWith, &groups(), Vec::new());
        let rows = s.rows();
        assert_eq!(rows.len(), 5);
        assert!(rows[0].header && rows[0].label == "ID_CIC_PRO_SKATERS");
        assert_eq!(s.entry(0), None);
        assert_eq!(s.entry(2).map(|e| e.label.as_str()), Some("#b"));
        let f = SpawnSkater::open(Mode::Freeskate, &groups(), Vec::new());
        assert_eq!(f.rows().len(), 7);
        assert_eq!(f.entry(1).map(|e| &e.skater), Some(&Skater::Own));
    }

    #[test]
    fn call_skater_fills_three_slots_then_shows_the_limit() {
        let rules = SpawnSkaterRules::default();
        let mut s = SpawnSkater::open(Mode::SkateWith, &groups(), Vec::new());
        for row in 1..=3 {
            assert!(matches!(s.select(&rules, row).actions[..], [Action::Call(_)]));
        }
        assert!(s.rows()[1].spawned);
        assert_eq!(s.select(&rules, 4).actions, [Action::LimitReached]);
        s.hilight(&rules, 2);
        assert_eq!(s.x(&rules).actions, [Action::Dismiss(Skater::Character("b".into()))]);
        assert_eq!(s.companions.len(), 2);
    }

    #[test]
    fn free_play_picks_previews_and_opens_options() {
        let rules = SpawnSkaterRules::default();
        let mut s = SpawnSkater::open(Mode::Freeskate, &groups(), Vec::new());
        assert_eq!(s.hilight(&rules, 3).actions, [Action::Preview(Skater::Character("a".into()))]);
        assert_eq!(s.select(&rules, 3).actions, [Action::SetPlayer(Skater::Character("a".into()))]);
        assert_eq!(s.select(&rules, 0).actions, []);
        assert_eq!(s.x(&rules).actions, [Action::Options]);
        assert_eq!(s.back(&rules).actions, [Action::Close]);
    }
}
