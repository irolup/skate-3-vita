//! The Skate Profile screen's natives (skate-game `offline_profile_host.rs`, PR #52 `b110bab`; doc 31 "Skate Profile",
//! research b308): the Progression object (registrar 0x825AFBA8) and the Stats object (registrar 0x825AB630) the movie
//! `profile/offline_skateProfile` reads its pages from. The tabs are `skate_core::offline_profile`; this host answers
//! the movie's calls from the page data and queues the screen states the C++ posts back (applied by the menu stage
//! after the call returns).
//!
//! skate_vita: no mods, so upstream's `ProfileMods` layers are not taken; the pages stay empty until a career system
//! fills them (as upstream's without a mod).
use crate::fe::apt_vm::{Value, Vm};
use skate_core::offline_profile::{OfflineProfile, ProfileData, ProfileOverall, ProfileRules, StatRow, StatValue};

#[derive(Clone, Default)]
pub struct OfflineProfileHost {
    pub screen: OfflineProfile,
    pub rules: ProfileRules,
    pub data: ProfileData,
    /// Screen states to post to the movie, in order (held until the movie loaded).
    pub states: Vec<i32>,
    /// The movie reported OnScreenLoaded.
    pub loaded: bool,
    /// The members the movie called (tests).
    #[cfg(test)]
    pub asked: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl OfflineProfileHost {
    /// Init 0x8264C530: tab 0 and its page.
    pub fn open(&mut self) {
        let (screen, states) = OfflineProfile::open(&self.rules);
        self.screen = screen;
        self.states = states;
        self.loaded = false;
    }

    /// A Progression / Stats member call; None for anything else.
    pub fn call(&self, vm: &mut Vm, object: &str, method: &str) -> Result<Option<Value>, String> {
        #[cfg(test)]
        if object == "Progression" || object == "Stats" {
            self.asked.lock().unwrap().push(format!("{object}.{method}"));
        }
        Ok(Some(match (object, method) {
            // 0x825AC888: the FE 61 screen's tab index (+0x3C).
            ("Stats", "GetCareerProfileSelectedTab") => Value::Number(self.screen.tab as f64),
            ("Stats", "GetStatsChunk") => rows(vm, &self.data.stats)?,
            ("Stats", "GetPersonalBestChunk") => rows(vm, &self.data.personal_bests)?,
            ("Progression", "GetOverallCareerData") => overall(vm, &self.data.career)?,
            ("Progression", "GetOverallChallengeData") => overall(vm, &self.data.challenges)?,
            // NOT RETAIL YET: no career milestones or summary groups exist in the engine; empty lists hide them.
            ("Progression", "GetMilestoneDotData" | "GetSummaryGroupData") => Value::Object(vm.array(Vec::new())?),
            _ => return Ok(None),
        }))
    }
}

/// [title, icon, current, total] (the movie clamps current to total).
fn overall(vm: &mut Vm, o: &ProfileOverall) -> Result<Value, String> {
    let items = vec![Value::Text(o.title.clone()), Value::Text(o.icon.clone()), Value::Number(o.current), Value::Number(o.total)];
    Ok(Value::Object(vm.array(items)?))
}

/// One [label, value] array per row.
fn rows(vm: &mut Vm, rows: &[StatRow]) -> Result<Value, String> {
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let value = match &r.value {
            StatValue::Number(n) => Value::Number(*n),
            StatValue::Text(t) => Value::Text(t.clone()),
        };
        out.push(Value::Object(vm.array(vec![Value::Text(r.label.clone()), value])?));
    }
    Ok(Value::Object(vm.array(out)?))
}
