//! skate_vita stand-in for the reference's `game_audio/mod_rules.rs` (the mods' mute / replace /
//! layer rules): no mods, so no rule set is ever made and nothing is muted.
use crate::mod_audio::EventRow;

pub(crate) struct RuleSet;

impl RuleSet {
    pub(crate) fn mutes(&self, _row: &EventRow) -> bool {
        false
    }
}
