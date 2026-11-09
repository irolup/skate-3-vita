//! Retail's team screen (FE 67, `TeamManagement`, template 104 `TeamSkater_Selection/team_management.swf`; TU3,
//! research of 7 Nov 2026, docs/customization-survey.md): the career team's player cards, the selected member's
//! options (Edit Skater, Change Name, Import Skater), the team's name (X) and logo (Y). The movie asks the natives
//! `TeamManagement.*` (registrar 825C3498) for its data; `SetState` picks its page.
//!
//! Code: enter 82651900, shown 82651A48, keys 82651EE0, highlight 82651E50, select 82651C70; natives GetTeamData
//! 825C36B0, GetSelectedTeammateData 825C38F0, GetEditSkaterOptions 825C39E8 (labels at 0x83025ECC).
//! NOT RETAIL YET: the logo page lists no logos (their textures are not baked); the members' photos are not rendered;
//! Import Skater does nothing (FE 0x44 is not ported).

/// Movie states (the movie's `State_*`).
pub const STATE_TEAMMATES: i32 = 10;
pub const STATE_TEAMMATE_INFO: i32 = 11;
pub const STATE_LOGOS: i32 = 20;
pub const STATE_TEAM_LOGO: i32 = 21;
pub const STATE_NEW_CAREER_LOGOS: i32 = 30;

/// The movie's player cards (`kNumPlayerCards`).
pub const CARDS: usize = 5;
/// The menu picker of a member's options (`kEditSkaterMenuId`): its highlights move nothing (82651E50).
pub const OPTIONS_MENU: i32 = 1;
/// A member's options when the member exists (0x83025ECC).
pub const OPTIONS: [&str; 3] = ["ID_CAC_EDITSKATER_TITLE", "ID_TEAM_MANAGEMENT_CHANGE_NAME", "ID_TEAM_MANAGEMENT_IMPORT_SKATER"];
/// Names are typed up to 16 characters (826397D8's last argument).
pub const NAME_LENGTH: usize = 16;

/// What the screen asks the game for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Edit Skater: the customisation on this member (826520B8 after the fade: FE 15, mode 0).
    EditSkater(usize),
    /// Change Name: the keyboard titled `ID_TEAM_MANAGEMENT_ENTER_NEW_TEAMMATE_NAME` on this member's name.
    RenameMember(usize),
    /// X, or OK on the empty-name warning: the keyboard titled `ID_TEAM_MANAGEMENT_ENTER_NEW_TEAM_NAME`.
    RenameTeam,
    /// Import Skater (FE 0x44).
    Import(usize),
    /// The team has no name (82651A48: popup 0x12E, `ID_TEAM_MANAGEMENT_EMPTY_TEAM_NAME`, OK).
    EmptyTeamName,
}

/// The keys the C++ handler takes (82651EE0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// B (Apt key 3).
    Back,
    /// Y (AptOption, 4): the logos.
    Logos,
    /// X (AptMisc, 5): the team's name.
    TeamName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Teammates,
    Logos,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    /// The team's name (the team record +0x360) and its members' names (member 0 is the player: the profile's name).
    pub name: String,
    pub members: Vec<String>,
    /// The selected card (0x83083A04).
    pub selected: usize,
    mode: Mode,
    /// Movie states to post, in order.
    pub states: Vec<i32>,
    pub sounds: Vec<&'static str>,
    pub effects: Vec<Effect>,
    /// B on the members: the screen closes.
    pub closed: bool,
}

impl Team {
    /// Opened on the career team (82651900); `shown` follows when the movie is in.
    pub fn open(name: String, members: Vec<String>) -> Self {
        Self { name, members, selected: 0, mode: Mode::Teammates, states: Vec::new(), sounds: Vec::new(), effects: Vec::new(), closed: false }
    }

    /// The movie is in (82651A48): the members, the selection, and a warning when the team has no name.
    pub fn shown(&mut self) {
        self.states.extend([STATE_TEAMMATES, STATE_TEAMMATE_INFO]);
        if self.name.is_empty() {
            self.effects.push(Effect::EmptyTeamName);
        }
    }

    /// `TeamManagement.GetTeamData` (825C36B0): the member count, the team's name, its logo, each member's photo.
    pub fn team_data(&self) -> (usize, &str) {
        (self.members.len(), &self.name)
    }

    /// `GetSelectedTeammateData` (825C38F0): the selection and its name (past the members, none).
    pub fn selected_data(&self) -> (usize, &str) {
        (self.selected, self.members.get(self.selected).map_or("", String::as_str))
    }

    /// `GetEditSkaterOptions` (825C39E8): a member's options, none on the logos or a locked card.
    pub fn options(&self) -> &'static [&'static str] {
        if self.mode == Mode::Teammates && self.selected < self.members.len() {
            &OPTIONS
        } else {
            &[]
        }
    }

    /// `Game.OnMenuHilight` (82651E50): the cards wrap over the members.
    pub fn highlight(&mut self, menu: i32, index: i32) {
        if self.mode != Mode::Teammates || menu == OPTIONS_MENU {
            return;
        }
        let n = self.members.len().max(1) as i32;
        self.selected = if index < 0 { (n - 1) as usize } else if index >= n { 0 } else { index as usize };
        self.states.push(STATE_TEAMMATE_INFO);
    }

    /// `Game.OnMenuSelect` (82651C70): a member's option.
    pub fn select(&mut self, option: i32) {
        if self.mode != Mode::Teammates || self.selected >= self.members.len() {
            return;
        }
        match option {
            0 => self.effects.push(Effect::EditSkater(self.selected)),
            1 => self.effects.push(Effect::RenameMember(self.selected)),
            2 => self.effects.push(Effect::Import(self.selected)),
            _ => {}
        }
    }

    /// One key of the C++ handler (82651EE0).
    pub fn key(&mut self, key: Key) -> bool {
        match key {
            Key::Back => {
                self.sounds.push("core_b_button");
                if self.mode == Mode::Logos {
                    self.mode = Mode::Teammates;
                    self.states.extend([STATE_TEAMMATES, STATE_TEAMMATE_INFO]);
                } else {
                    self.closed = true;
                }
                true
            }
            Key::Logos if self.mode == Mode::Teammates => {
                self.sounds.push("core_a_button");
                self.mode = Mode::Logos;
                self.states.push(STATE_LOGOS);
                true
            }
            Key::TeamName if self.mode == Mode::Teammates => {
                self.effects.push(Effect::RenameTeam);
                true
            }
            _ => false,
        }
    }

    /// A name typed (826521E0 for the team, 826522A0 for a member): kept when it changed, then the page shows it.
    pub fn named(&mut self, member: Option<usize>, text: &str) {
        let text: String = text.chars().take(NAME_LENGTH).collect();
        match member {
            None => self.name = text,
            Some(m) => {
                if let Some(n) = self.members.get_mut(m) {
                    *n = text;
                }
            }
        }
        self.states.extend([STATE_TEAMMATES, STATE_TEAMMATE_INFO]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team() -> Team {
        let mut t = Team::open("Rippers".into(), vec!["Me".into()]);
        t.shown();
        t
    }

    #[test]
    fn the_members_show_with_the_selected_ones_options() {
        let t = team();
        assert_eq!(t.states, [STATE_TEAMMATES, STATE_TEAMMATE_INFO]);
        assert_eq!((t.team_data(), t.selected_data(), t.options().len()), ((1, "Rippers"), (0, "Me"), 3));
        assert!(t.effects.is_empty());
    }

    #[test]
    fn a_team_without_a_name_asks_for_one() {
        let mut t = Team::open(String::new(), vec!["Me".into()]);
        t.shown();
        assert_eq!(t.effects, [Effect::EmptyTeamName]);
        t.named(None, "A name longer than sixteen");
        assert_eq!(t.name, "A name longer th");
    }

    #[test]
    fn the_cards_wrap_over_the_members_and_options_open_the_customisation_or_the_keyboard() {
        let mut t = Team::open("T".into(), vec!["Me".into(), "Two".into()]);
        t.highlight(0, 1);
        assert_eq!(t.selected_data(), (1, "Two"));
        t.highlight(0, 2);
        assert_eq!(t.selected, 0);
        t.highlight(0, -1);
        assert_eq!(t.selected, 1);
        // The options' own menu moves no card.
        t.highlight(OPTIONS_MENU, 0);
        assert_eq!(t.selected, 1);
        t.select(0);
        t.select(1);
        assert_eq!(t.effects, [Effect::EditSkater(1), Effect::RenameMember(1)]);
    }

    #[test]
    fn y_opens_the_logos_b_comes_back_then_closes_and_x_names_the_team() {
        let mut t = team();
        assert!(t.key(Key::Logos));
        assert_eq!((t.states.last(), t.options().len()), (Some(&STATE_LOGOS), 0));
        assert!(!t.key(Key::TeamName), "not on the logos");
        t.key(Key::Back);
        assert!(!t.closed);
        assert!(t.key(Key::TeamName));
        assert_eq!(t.effects, [Effect::RenameTeam]);
        t.key(Key::Back);
        assert!(t.closed);
    }
}
