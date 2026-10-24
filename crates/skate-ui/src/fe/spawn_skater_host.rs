//! FE 62 SpawnSkater's `SkaterSpawner` natives (registrar sub_825B8F40, b543) and its picker callbacks while it is on
//! top (skate-game `spawn_skater_host.rs`, PR #52 `36018e19`; doc 31 "SpawnSkater (FE 62)"). The stage applies what
//! this host queues (screen states, actions, sounds) after the movie call returns.
//!
//! skate_vita: the groups come from the living world's skater profiles, not upstream's character library
//! (`custom_models`, no player character swap on the Vita): Free Play lists only You, Call Skater the pros retail
//! keeps for it (owner, 2026-10-24).
use crate::fe::apt_vm::{to_integer, Value, Vm};
use skate_core::spawn_skater::{Action, Group, SpawnSkater, SpawnSkaterRules};

#[derive(Clone, Default)]
pub struct SpawnSkaterHost {
    pub screen: SpawnSkater,
    pub rules: SpawnSkaterRules,
    /// FE 62 is on top (its picker callbacks are ours).
    pub open: bool,
    /// Screen states to post to the movie, in order.
    pub states: Vec<i32>,
    /// What the picks and keys asked of the game, in order.
    pub actions: Vec<Action>,
    pub sounds: Vec<String>,
}

impl SpawnSkaterHost {
    pub fn call(&mut self, vm: &mut Vm, object: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        if !self.open {
            return Ok(None);
        }
        let row = || args.get(1).map_or(-1, to_integer);
        Ok(Some(match (object, method) {
            // 825B8FE8: [rowCount, names[], isHeader[], spawned[]].
            ("SkaterSpawner", "GetSkatersData") => {
                let rows = self.screen.rows();
                let names = vm.array(rows.iter().map(|r| Value::Text(r.label.clone())).collect())?;
                let headers = vm.array(rows.iter().map(|r| Value::Bool(r.header)).collect())?;
                let spawned = vm.array(rows.iter().map(|r| Value::Bool(r.spawned)).collect())?;
                let data = vec![Value::Number(rows.len() as f64), Value::Object(names), Value::Object(headers), Value::Object(spawned)];
                Value::Object(vm.array(data)?)
            }
            ("SkaterSpawner", "GetScreenMode") => Value::Number(self.screen.mode.code() as f64),
            ("SkaterSpawner", "GetLastMenuHighlightIndex") => Value::Number(self.screen.highlight.map_or(-1.0, |h| h as f64)),
            // NOT RETAIL YET (inferred): header rows are not pickable.
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(row() >= 0 && self.screen.entry(row() as usize).is_some()),
            ("Game", "OnMenuHilight") => {
                if row() >= 0 {
                    let out = self.screen.hilight(&self.rules, row() as usize);
                    self.take(out);
                }
                Value::Undefined
            }
            ("Game", "OnMenuSelect") => {
                if row() >= 0 {
                    let out = self.screen.select(&self.rules, row() as usize);
                    self.take(out);
                }
                Value::Undefined
            }
            _ => return Ok(None),
        }))
    }

    pub fn take(&mut self, out: skate_core::spawn_skater::Outcome) {
        self.states.extend(out.states);
        self.actions.extend(out.actions);
        self.sounds.extend(out.sound);
    }
}

/// A pro Call Skater can list: its character key and its name's string id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Callable {
    pub key: String,
    pub name_id: String,
}

/// The pros of the living world's skater profiles (`private/living_world/skater_profiles.json`) Call Skater lists:
/// pro-class records in the free-roam pool with attribute 0xCABC362B1B05CA19 (retail mode 100 keeps those, b544;
/// upstream stands in pool membership for the attribute). Profile order.
pub fn callable(profiles: &[u8]) -> Vec<Callable> {
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(profiles) else { return Vec::new() };
    let chars = &doc["characters"];
    doc["free_roam_pool"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| k.as_str())
        .filter(|k| chars[k]["parent"] == "pro_skaters" && chars[k]["fields"]["Hash_CABC362B1B05CA19"] == true)
        .filter_map(|k| Some(Callable { key: k.to_owned(), name_id: chars[k]["name_id"].as_str()?.to_owned() }))
        .collect()
}

/// skate_vita: the limit popup's text for a cap other than retail's 3 (`ID_CALL_SKATER_LIMIT_REACHED` says "There are
/// already three skaters called. Please dismiss one before calling another."), as a literal (`#`); None at 3.
pub fn limit_text(limit: usize) -> Option<String> {
    match limit {
        3 => None,
        1 => Some("#There is already a skater called. Please dismiss them before calling another.".into()),
        n => Some(format!("#There are already {n} skaters called. Please dismiss one before calling another.")),
    }
}

/// The groups: You (Free Play, retail save slot 0) and the callable pros (Call Skater). Retail headers (b544).
/// skate_vita: no pros / IP / imported groups in Free Play (no player character swap on the Vita).
pub fn groups(callable: &[Callable]) -> Vec<Group> {
    use skate_core::spawn_skater::{Entry, Mode, Skater};
    // NOT RETAIL YET: the You row's label (82558BD0's vt+0xC) is not read.
    let you = Group {
        id: "you".into(),
        header: "ID_FREESKATE_YOU_HEADER".into(),
        modes: vec![Mode::Freeskate],
        entries: vec![Entry { skater: Skater::Own, label: "#You".into() }],
    };
    let call = Group {
        id: "pros_call".into(),
        header: "ID_CIC_PRO_SKATERS".into(),
        modes: vec![Mode::SkateWith],
        entries: callable.iter().map(|c| Entry { skater: Skater::Character(c.key.clone()), label: c.name_id.clone() }).collect(),
    };
    vec![you, call]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callable_keeps_pros_with_the_retail_attribute_in_the_pool() {
        let json = br#"{"free_roam_pool":["a","b","c","d"],"characters":{
            "a":{"parent":"pro_skaters","name_id":"ID_A","fields":{"Hash_CABC362B1B05CA19":true}},
            "b":{"parent":"pro_skaters","name_id":"ID_B","fields":{"Hash_CABC362B1B05CA19":false}},
            "c":{"parent":"community_skaters","name_id":"ID_C","fields":{"Hash_CABC362B1B05CA19":true}},
            "d":{"parent":"pro_skaters","name_id":"ID_D","fields":{"Hash_CABC362B1B05CA19":true}},
            "e":{"parent":"pro_skaters","name_id":"ID_E","fields":{"Hash_CABC362B1B05CA19":true}}}}"#;
        let keys: Vec<_> = callable(json).into_iter().map(|c| c.key).collect();
        assert_eq!(keys, ["a", "d"]);
        let g = groups(&callable(json));
        assert_eq!(g[1].entries[1].label, "ID_D");
    }

    #[test]
    fn the_limit_text_follows_the_cap() {
        assert_eq!(limit_text(3), None);
        assert!(limit_text(1).unwrap().starts_with("#There is already a skater called."));
        assert!(limit_text(2).unwrap().contains(" 2 skaters "));
    }
}
