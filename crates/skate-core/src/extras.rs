//! Free Play pause row Extras (doc 31 "Extras and cheat codes"): retail FE screen 18 (vtable 0x82305390, template 15
//! Extras, `main/extras.swf`, research b320). The movie lists the rows through Game.Extras_GetNumOptions /
//! _GetOptionTitle / _MenuTitle and asks Game.OnMenuGetItemEnabled / OnMenuSelect; the C++ side keeps the list mode
//! (main rows or the Skate Reel sub-list, +0x84):
//! - Back (input 0x82655658, never consumed): in the sub-list plays core_b_button, goes back to the main rows and posts
//!   SetState(10); in the main rows plays core_b_button and pops the screen;
//! - select (0x826558E8): row 0 opens text entry FE 46 for a cheat code, rows 1-3 redeem console codes, row 4 pushes
//!   Credits FE 25, row 5 shows the licenses in the text viewer FE 3, row 6 opens the Skate Reel sub-list.
//!
//! What the console-only rows do on PC is ours (user 2026-10-19): rows 1-3 unlock their content directly, Licenses shows
//! this project's licenses, Credits stays greyed until FE 25 is ported. Rows are data (`ExtrasRules`), retail's the
//! defaults; a mod adds or hides rows.

/// One entry of the profile unlock list (retail `sub_82503C30(list, kind, fnv1(name), ..)`, research b321; the hash
/// is FNV-1, multiply then xor, b458, not FNV-1a as b321 said): kind `entitlement` (retail 8: a redeemed code),
/// `sponsor` (6: a clothing / sponsor item, `cac_unlock_sponsors`), `location` (0: a world location), `challenge`
/// (passed to TheSim+0xE0 vt+0x14). Retail kinds (b458, string table 8302CC7C): 0 LOCATIONS, 1 CHARACTERS, 2 REPLAY,
/// 3 EGGS, 4 GESTURES, 5 CLOTHING_UNLOCK, 6 CLOTHING_SPONSOR, 7 SKATEPARK_SET, 8 ENTITLEMENTS, 9 WORLDS, 10 CHALLENGES,
/// 11 MOVIE, 12 DLC_PRODUCT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unlock {
    pub kind: String,
    pub name: String,
}

impl Unlock {
    pub fn new(kind: &str, name: &str) -> Self {
        Self { kind: kind.into(), name: name.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtrasAction {
    /// Open text entry for a cheat code (row 0).
    CheatEntry,
    /// Grant content directly (ours for rows 1-3, user 2026-10-19): retail's redeem (`sub_82700380`) sets the row's
    /// entitlement flag, then grants. The engine has no unlock store yet and nothing is locked ("leave everything
    /// unlocked for now"): the grants go out as events for the store to take when it exists.
    Grant { redeem: Unlock, grants: Vec<Unlock> },
    /// Push another FE screen by key.
    PushScreen(String),
    /// Show a text in the viewer (row 5: the licenses).
    TextViewer { title: String, body: String },
    /// Open the Skate Reel sub-list (row 6).
    SkateReel,
    /// A mod's row: its select event.
    Mod(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtrasRow {
    /// Stable id (`extras.cheat_code` ..).
    pub id: String,
    pub label: String,
    pub action: ExtrasAction,
    /// Greyed (not ported, or redeemed).
    pub greyed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtrasRules {
    pub rows: Vec<ExtrasRow>,
    /// The movie's list state (AS SetState(10) builds the list).
    pub list_state: i32,
    pub title: String,
    pub reel_title: String,
    pub sound_back: String,
}

impl Default for ExtrasRules {
    fn default() -> Self {
        let row = |id: &str, label: &str, action, greyed| ExtrasRow { id: id.into(), label: label.into(), action, greyed };
        let grant = |redeem: &str, grants: &[(&str, &str)]| ExtrasAction::Grant {
            redeem: Unlock::new("entitlement", redeem),
            grants: grants.iter().map(|(k, n)| Unlock::new(k, n)).collect(),
        };
        Self {
            rows: vec![
                row("extras.cheat_code", "ID_EXTRA_CHEAT_CODE", ExtrasAction::CheatEntry, false),
                // Ours: unlock directly (user 2026-10-19). Grants from b321: the row key (kind 8) and what the redeem
                // grants (Emerica 827012E8, Etnies 82701578, pre-sell 82701FF0 + the two `code_unlocks/blackboxpark`
                // challenges, names not in our export: their record keys). Retail greys a redeemed row; with no store
                // they stay enabled.
                row("extras.emerica_shoe", "ID_EXTRA_EMERICA_SHOE_CODE", grant("EMERICA_SHOE_UNLOCK", &[("sponsor", "cac_shoe_emerica_promo")]), false),
                row("extras.etnies_shoe", "ID_EXTRA_ETNIES_SHOE_CODE", grant("ETNIES_SHOE_UNLOCK", &[("sponsor", "cac_shoe_etnies_promo")]), false),
                row(
                    "extras.pre_sell",
                    "ID_EXTRA_PRE_SELL_CODE",
                    grant("BLACKBOX_SKATE_PARK", &[("location", "dist_blackboxpark"), ("challenge", "Hash_E464A929BC793D07"), ("challenge", "Hash_EB6B03C256ACDD4B")]),
                    false,
                ),
                // FE 25 (template 30 `extras/credits.swf`) is not ported: greyed (user 2026-10-19).
                row("extras.credits", "ID_EXTRA_CREDITS", ExtrasAction::PushScreen("credits".into()), true),
                // Ours: this project's licenses in the text viewer FE 3 (b322). WIP, greyed for now (user 2026-10-19:
                // "skip it and grey it out for now"; "mark it as WIP and we can come back to it"). Left to check: the
                // long licenses text with the raised VM budget (retail_text_viewer_shows_the_licenses).
                row("extras.licenses", "ID_EXTRA_LICENSES", ExtrasAction::TextViewer { title: "ID_EXTRA_LICENSES_TITLE".into(), body: "licenses".into() }, true),
                // FE 24 (video player) and the reel videos are not ported: greyed.
                row("extras.skate_reel", "ID_SKATE_REEL_VIDEOS", ExtrasAction::SkateReel, true),
            ],
            list_state: 10,
            title: "ID_MAIN_MENU_EXTRAS".into(),
            reel_title: "ID_SKATE_REEL_VIDEOS".into(),
            sound_back: "core_b_button".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExtrasMode {
    #[default]
    Main,
    SkateReel,
}

/// What Back does (0x82655658).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackOutcome {
    pub sound: String,
    /// None = pop the screen.
    pub state: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub mode: ExtrasMode,
}

impl Extras {
    /// Init 0x82655478: the main rows.
    pub fn open() -> Self {
        Self::default()
    }

    pub fn title<'a>(&self, rules: &'a ExtrasRules) -> &'a str {
        match self.mode {
            ExtrasMode::Main => &rules.title,
            ExtrasMode::SkateReel => &rules.reel_title,
        }
    }

    pub fn back(&mut self, rules: &ExtrasRules) -> BackOutcome {
        let state = match self.mode {
            ExtrasMode::SkateReel => {
                self.mode = ExtrasMode::Main;
                Some(rules.list_state)
            }
            ExtrasMode::Main => None,
        };
        BackOutcome { sound: rules.sound_back.clone(), state }
    }

    /// OnMenuGetItemEnabled (0x82655830) for the main rows.
    pub fn enabled(&self, rules: &ExtrasRules, row: usize) -> bool {
        rules.rows.get(row).is_some_and(|r| !r.greyed)
    }

    /// OnMenuSelect (0x826558E8): the row's action (None for a greyed or missing row).
    pub fn select(&mut self, rules: &ExtrasRules, row: usize) -> Option<ExtrasAction> {
        let r = rules.rows.get(row).filter(|r| !r.greyed)?;
        if r.action == ExtrasAction::SkateReel {
            self.mode = ExtrasMode::SkateReel;
        }
        Some(r.action.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_leaves_the_sub_list_then_closes() {
        let mut rules = ExtrasRules::default();
        rules.rows[6].greyed = false;
        let mut e = Extras::open();
        assert_eq!(e.select(&rules, 6), Some(ExtrasAction::SkateReel));
        assert_eq!(e.title(&rules), "ID_SKATE_REEL_VIDEOS");
        assert_eq!(e.back(&rules), BackOutcome { sound: "core_b_button".into(), state: Some(10) });
        assert_eq!(e.mode, ExtrasMode::Main);
        assert_eq!(e.back(&rules).state, None, "the main rows close the screen");
    }

    #[test]
    fn greyed_rows_do_nothing_and_the_cheat_row_opens_entry() {
        let rules = ExtrasRules::default();
        let mut e = Extras::open();
        assert_eq!(rules.rows.len(), 7, "retail's seven rows");
        assert!(e.enabled(&rules, 0));
        assert_eq!(e.select(&rules, 0), Some(ExtrasAction::CheatEntry));
        assert!(!e.enabled(&rules, 4));
        assert_eq!(e.select(&rules, 4), None);
        // Rows 1-3 grant directly (nothing locked yet).
        match e.select(&rules, 1) {
            Some(ExtrasAction::Grant { redeem, grants }) => {
                assert_eq!(redeem, Unlock::new("entitlement", "EMERICA_SHOE_UNLOCK"));
                assert_eq!(grants, [Unlock::new("sponsor", "cac_shoe_emerica_promo")]);
            }
            other => panic!("row 1: {other:?}"),
        }
        assert_eq!(e.select(&rules, 9), None);
    }
}
