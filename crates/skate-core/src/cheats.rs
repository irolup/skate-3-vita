//! Cheat codes (doc 31 "Extras and cheat codes", research b304): the text from the Extras cheat row's entry goes to
//! the matcher `sub_82656080`, which walks the cheat table (0x83025F00, `{code, id, title, enabled text, disabled
//! text}`) with a plain case-sensitive strcmp (`sub_82AE89B0`) over the first 32 characters (callback 0x82655F98).
//! - no match: dialog ID_VALIDATE_CODE_TITLE / ID_UNLOCK_CODE_FAILED / ID_COMMON_CONTINUE;
//! - a mode-gated code (zombie, id 5) outside its modes: ID_INVALID_CODE_TITLE / the row's refusal text / ID_COMMON_OK,
//!   nothing changes;
//! - else the cheat TOGGLES (entering it again turns it off) and the dialog is the row's title with its enabled or
//!   disabled text / ID_COMMON_OK.
//!
//! The table is data (`CheatRules`), retail's rows as defaults, so a mod can add codes (its own ids) and the engine
//! keeps the on / off state ([`CheatState`]) as host state the cheat event changes.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheatRow {
    /// The code as typed (case-sensitive).
    pub code: String,
    /// Stable id the engine acts on (`zombie`; retail id 5).
    pub id: String,
    pub title: String,
    pub enabled_text: String,
    pub disabled_text: String,
    /// Modes the code may be entered in (`MenuContext::mode`); empty = any. Refused elsewhere with `refused_text`.
    pub modes: Vec<String>,
    pub refused_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub title: String,
    pub text: String,
    pub button: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheatRules {
    pub rows: Vec<CheatRow>,
    /// Characters the matcher keeps (the callback copies at most 32).
    pub max_chars: usize,
    pub no_match: Dialog,
    pub refused_title: String,
    pub ok_button: String,
}

impl Default for CheatRules {
    fn default() -> Self {
        Self {
            rows: vec![CheatRow {
                // Table entry 3 (0x821F9438 / 0x821F9440 / 0x821F9454 / 0x821F9474). NOT RETAIL YET: retail allows it
                // when the mode word 0x830B7AE8 is 0, 3 or 4 (values not decoded); our menus' only mode is Free Play.
                // The other retail rows (streetsweeper id 4, ids 0-3 and 6-9) change things the engine does not have
                // yet; they are not listed.
                code: "zombie".into(),
                id: "zombie".into(),
                title: "ID_VALID_CODE_TITLE".into(),
                enabled_text: "ID_VALID_CODE_CHEAT_ZOMBIEMODE".into(),
                disabled_text: "ID_VALID_CODE_CHEAT_ZOMBIEMODE_DISABLED".into(),
                modes: vec!["FreePlay".into()],
                refused_text: "ID_INVALID_CODE_CHEAT_ZOMBIEMODE".into(),
            }],
            max_chars: 32,
            no_match: Dialog { title: "ID_VALIDATE_CODE_TITLE".into(), text: "ID_UNLOCK_CODE_FAILED".into(), button: "ID_COMMON_CONTINUE".into() },
            refused_title: "ID_INVALID_CODE_TITLE".into(),
            ok_button: "ID_COMMON_OK".into(),
        }
    }
}

/// Which cheats are on (host state).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheatState {
    pub on: std::collections::BTreeSet<String>,
}

impl CheatState {
    pub fn is_on(&self, id: &str) -> bool {
        self.on.contains(id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheatOutcome {
    NoMatch,
    Refused { id: String },
    /// The cheat's new state.
    Toggled { id: String, on: bool },
}

/// The matcher: what happened and the dialog to show.
pub fn enter(text: &str, rules: &CheatRules, mode: &str, state: &mut CheatState) -> (CheatOutcome, Dialog) {
    let text: String = text.chars().take(rules.max_chars).collect();
    let Some(row) = rules.rows.iter().find(|r| r.code == text) else {
        return (CheatOutcome::NoMatch, rules.no_match.clone());
    };
    if !row.modes.is_empty() && !row.modes.iter().any(|m| m == mode) {
        let dialog = Dialog { title: rules.refused_title.clone(), text: row.refused_text.clone(), button: rules.ok_button.clone() };
        return (CheatOutcome::Refused { id: row.id.clone() }, dialog);
    }
    let on = !state.on.remove(&row.id);
    if on {
        state.on.insert(row.id.clone());
    }
    let text = if on { &row.enabled_text } else { &row.disabled_text };
    let dialog = Dialog { title: row.title.clone(), text: text.clone(), button: rules.ok_button.clone() };
    (CheatOutcome::Toggled { id: row.id.clone(), on }, dialog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zombie_toggles_and_shows_its_dialogs() {
        let rules = CheatRules::default();
        let mut state = CheatState::default();
        let (out, dialog) = enter("zombie", &rules, "FreePlay", &mut state);
        assert_eq!(out, CheatOutcome::Toggled { id: "zombie".into(), on: true });
        assert_eq!((dialog.title.as_str(), dialog.text.as_str(), dialog.button.as_str()), ("ID_VALID_CODE_TITLE", "ID_VALID_CODE_CHEAT_ZOMBIEMODE", "ID_COMMON_OK"));
        assert!(state.is_on("zombie"));
        let (out, dialog) = enter("zombie", &rules, "FreePlay", &mut state);
        assert_eq!(out, CheatOutcome::Toggled { id: "zombie".into(), on: false }, "entering it again turns it off");
        assert_eq!(dialog.text, "ID_VALID_CODE_CHEAT_ZOMBIEMODE_DISABLED");
    }

    #[test]
    fn the_match_is_case_sensitive_and_unknown_codes_fail() {
        let rules = CheatRules::default();
        let mut state = CheatState::default();
        let (out, dialog) = enter("Zombie", &rules, "FreePlay", &mut state);
        assert_eq!(out, CheatOutcome::NoMatch);
        assert_eq!(dialog.text, "ID_UNLOCK_CODE_FAILED");
        assert_eq!(enter("", &rules, "FreePlay", &mut state).0, CheatOutcome::NoMatch);
        assert!(state.on.is_empty());
    }

    #[test]
    fn a_gated_code_is_refused_outside_its_modes() {
        let rules = CheatRules::default();
        let mut state = CheatState::default();
        let (out, dialog) = enter("zombie", &rules, "Career", &mut state);
        assert_eq!(out, CheatOutcome::Refused { id: "zombie".into() });
        assert_eq!((dialog.title.as_str(), dialog.text.as_str()), ("ID_INVALID_CODE_TITLE", "ID_INVALID_CODE_CHEAT_ZOMBIEMODE"));
        assert!(state.on.is_empty(), "nothing changes");
    }

    #[test]
    fn only_the_first_32_characters_count() {
        let mut rules = CheatRules::default();
        rules.rows[0].code = "a".repeat(32);
        let mut state = CheatState::default();
        assert!(matches!(enter(&"a".repeat(40), &rules, "FreePlay", &mut state).0, CheatOutcome::Toggled { .. }));
    }
}
