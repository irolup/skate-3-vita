//! The stock mood tables from setup data (`private/living_world/tables.json`, classes
//! `livingworld_moodeventcategories`, `livingworld_entities_moodreactions`,
//! `livingworld_entities_moodresults`, `livingworld_entities`) into
//! `skate_core::living_world::peds::mood::MoodTables`, parents resolved (doc 26 "Ped behaviour
//! runtime"). Field bindings: `.local/research/peds/b6-ped-mood-fields.md`.

use serde_json::Value;
use skate_core::living_world::peds::chase::ChaseRecord;
use skate_core::living_world::peds::conversation::ConversationRow;
use skate_core::living_world::peds::perception::Sight;
use skate_core::living_world::peds::takedown::{TakedownEntry, TakedownTable};
use skate_core::living_world::peds::mood::{MoodCategory, MoodReactions, MoodResult, MoodTables, Prerequisite};
use std::collections::BTreeMap;

/// eMoodResult order (the debug list at `0x820646BC`); the graph's `want` attributes use these.
pub(crate) const WANTS: [&str; 17] = [
    "none", "angrychase", "warn", "taunt", "returngreet", "greet", "startconversation", "alertto", "joinchase", "throwhandprop", "flee", "startle", "taze",
    "nearbycollisionreaction", "slamreaction", "startspectate", "nearbyskatertrick",
];

/// Which mood reactions an entity type uses (`livingworld_entities` field).
pub(crate) const REACTIONS_FIELD: &str = "Hash_712E5451373663BE";

struct Class<'a>(&'a serde_json::Map<String, Value>);

impl<'a> Class<'a> {
    /// A field by hash prefix (`Hash_FD19`) or name, walking the parents.
    fn field(&self, key: &str, name: &str) -> Option<&'a Value> {
        let mut k = Some(key.to_string());
        let mut guard = 0;
        while let Some(x) = k {
            let rec = self.0.get(&x)?;
            if let Some(fields) = rec.get("fields").and_then(Value::as_object) {
                if let Some(v) = fields.iter().find(|(f, _)| f.as_str() == name || f.starts_with(name)).map(|(_, v)| v).filter(|v| !v.is_null()) {
                    return Some(v);
                }
            }
            guard += 1;
            if guard > 32 {
                return None;
            }
            k = rec.get("parent").and_then(Value::as_str).map(str::to_string);
        }
        None
    }
    fn f32(&self, key: &str, name: &str) -> Option<f32> {
        self.field(key, name).and_then(Value::as_f64).map(|v| v as f32)
    }
    fn u32(&self, key: &str, name: &str) -> Option<u32> {
        self.field(key, name).and_then(Value::as_u64).map(|v| v as u32)
    }
    fn flag(&self, key: &str, name: &str) -> bool {
        self.field(key, name).and_then(Value::as_bool).unwrap_or(false)
    }
    fn reference(&self, key: &str, name: &str) -> Option<String> {
        self.field(key, name).and_then(|v| v.get("key")).and_then(Value::as_str).map(str::to_string)
    }
}

/// Parse the mood tables from `tables.json` bytes.
#[cfg(test)]
pub(crate) fn parse(bytes: &[u8]) -> Result<MoodTables, String> {
    let root: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    parse_in(&root)
}

/// The same from the parsed `tables.json`.
pub(crate) fn parse_in(root: &Value) -> Result<MoodTables, String> {
    let classes = root.get("classes").and_then(Value::as_object).ok_or("tables.json has no classes")?;
    let class = |name: &str| classes.get(name).and_then(Value::as_object).map(Class).ok_or_else(|| format!("tables.json has no {name}"));
    let cats = class("livingworld_moodeventcategories")?;
    let results = class("livingworld_entities_moodresults")?;
    let reactions = class("livingworld_entities_moodreactions")?;
    let entities = class("livingworld_entities")?;
    let mut t = MoodTables::default();
    for key in cats.0.keys() {
        // B1DD absent (discrete): the attribute default 0.0 (`0x830D0850`).
        t.categories.insert(key.clone(), MoodCategory { magnitude: cats.f32(key, "Hash_B1DD").unwrap_or(0.0), lifetime: cats.f32(key, "Hash_A379").unwrap_or(30.0), near: cats.f32(key, "Hash_D16BF47CF5417C44").unwrap_or(0.0), far: cats.f32(key, "Hash_A7EB37BC10C5A42F").unwrap_or(0.0), priority: cats.u32(key, "Hash_1A3FB63DFDAE0C4F").unwrap_or(0) });
    }
    for key in results.0.keys() {
        let gate = |flag: &str| results.flag(key, flag);
        let r = MoodResult {
            category: results.reference(key, "Hash_B45E"),
            instigator_type: gate("Hash_470C").then(|| results.reference(key, "Hash_1C72")).flatten(),
            second_type: gate("Hash_FD19").then(|| results.reference(key, "Hash_1770")).flatten(),
            magnitude_at_least: gate("Hash_C404").then(|| results.f32(key, "Hash_8239").unwrap_or(0.0)),
            count_at_least: gate("Hash_B1A9").then(|| results.u32(key, "Hash_0982").unwrap_or(0)),
            hand_prop: gate("Hash_072D").then(|| results.flag(key, "Hash_ADAC")),
            target_check_a: gate("Hash_D9F3").then(|| results.flag(key, "Hash_AA82")),
            target_check_b: gate("Hash_D44E").then(|| results.flag(key, "Hash_9B44")),
            outstanding: gate("Hash_7FF7").then(|| (results.u32(key, "Hash_9F09").unwrap_or(0), results.u32(key, "Hash_C41A").unwrap_or(u32::MAX))),
            roll: gate("Hash_9ABC").then(|| (results.f32(key, "Hash_093E").unwrap_or(0.0), results.f32(key, "Hash_441C").unwrap_or(0.0))),
            target_source: results.u32(key, "Hash_C69F").unwrap_or(0),
            want_flag: gate("Hash_CE65"),
            suppress: results.f32(key, "Hash_4EC2").unwrap_or(0.0),
            wants: results
                .field(key, "Hash_4694")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).filter_map(|i| WANTS.get(i as usize)).map(|w| w.to_string()).collect())
                .unwrap_or_default(),
        };
        t.results.insert(key.clone(), r);
    }
    for key in reactions.0.keys() {
        let list = |name: &str| reactions.field(key, name).and_then(Value::as_array).cloned().unwrap_or_default();
        let r = MoodReactions {
            results: list("results")
                .iter()
                .filter_map(|e| Some((e.get("ref")?.get("key")?.as_str()?.to_string(), e.get("priority")?.as_u64()? as u32)))
                .collect(),
            prerequisites: list("prerequisites")
                .iter()
                .filter_map(|e| {
                    Some(Prerequisite {
                        category: e.get("event")?.get("key")?.as_str()?.to_string(),
                        kind: e.get("u32_24")?.as_u64()? as u32,
                        comparator: e.get("u32_28")?.as_u64()? as u32,
                        value: f32::from_bits(e.get("u32_32")?.as_u64()? as u32),
                    })
                })
                .collect(),
        };
        t.reactions.insert(key.clone(), r);
    }
    for (key, rec) in entities.0 {
        if let Some(p) = rec.get("parent").and_then(Value::as_str) {
            t.entity_parents.insert(key.clone(), p.to_string());
        }
    }
    Ok(t)
}

/// Which chase record an entity type uses (`livingworld_entities` field).
pub(crate) const CHASE_FIELD: &str = "Hash_FBC42525E74D946F";

/// The chase record every chase group takes (`82D967B0`, b105): `zombie` while the zombie cheat is on (key hash
/// 0x69FABBFD81C0E5A7 built at `82D96860`), else `global` (`*(0x830C7200)`, static init `82F8FEB8`) [code].
pub(crate) const GROUP_CHASE_KEY: &str = "global";
/// The zombie cheat's chase and takedown records: the group above, and every ped's own chase record (`ped+5712`) and
/// takedown table (`ped+5728`) in the ped constructor `82E33198` (`82E33FAC`..`82E3402C`) [code].
pub(crate) const ZOMBIE_KEY: &str = "zombie";

/// The chase records (`livingworld_entities_chase`, parents merged, numeric fields) by entity
/// type, and the `global` and `zombie` records.
#[cfg(test)]
pub(crate) fn chase_records(bytes: &[u8]) -> (BTreeMap<String, ChaseRecord>, Option<ChaseRecord>, Option<ChaseRecord>) {
    let Ok(root) = serde_json::from_slice::<Value>(bytes) else { return Default::default() };
    chase_records_in(&root)
}

/// The same from the parsed `tables.json` (parsed once per load, shared by every reader).
pub(crate) fn chase_records_in(root: &Value) -> (BTreeMap<String, ChaseRecord>, Option<ChaseRecord>, Option<ChaseRecord>) {
    let (Some(entities), Some(chase)) = (root.pointer("/classes/livingworld_entities").and_then(Value::as_object), root.pointer("/classes/livingworld_entities_chase").and_then(Value::as_object)) else {
        return Default::default();
    };
    let record = |key: &str| -> Option<ChaseRecord> {
        let mut chain = Vec::new();
        let mut k = Some(key.to_string());
        while let Some(x) = k.filter(|_| chain.len() < 32) {
            let rec = chase.get(&x)?;
            chain.push(rec);
            k = rec.get("parent").and_then(Value::as_str).map(str::to_string);
        }
        let mut r = ChaseRecord::default();
        // Parents first, the record's own fields last.
        for rec in chain.iter().rev() {
            for (f, v) in rec.get("fields").and_then(Value::as_object).into_iter().flatten() {
                if let Some(n) = v.as_f64() {
                    r.fields.insert(f.clone(), n as f32);
                }
            }
        }
        Some(r)
    };
    let c = Class(entities);
    let by_entity = entities.keys().filter_map(|e| Some((e.clone(), record(&c.reference(e, CHASE_FIELD)?)?))).collect();
    (by_entity, record(GROUP_CHASE_KEY), record(ZOMBIE_KEY))
}

/// Which takedown table an entity type uses (`livingworld_entities` field).
pub(crate) const TAKEDOWN_FIELD: &str = "Hash_76AC331FBD5803FF";

/// The takedown tables (`livingworld_entity_takedown`, parents merged) by entity type, and the `zombie` table.
#[cfg(test)]
pub(crate) fn takedown_tables(bytes: &[u8]) -> (BTreeMap<String, TakedownTable>, Option<TakedownTable>) {
    let Ok(root) = serde_json::from_slice::<Value>(bytes) else { return Default::default() };
    takedown_tables_in(&root)
}

/// The same from the parsed `tables.json` (parsed once per load, shared by every reader).
pub(crate) fn takedown_tables_in(root: &Value) -> (BTreeMap<String, TakedownTable>, Option<TakedownTable>) {
    let (Some(entities), Some(tables)) = (root.pointer("/classes/livingworld_entities").and_then(Value::as_object), root.pointer("/classes/livingworld_entity_takedown").and_then(Value::as_object)) else {
        return Default::default();
    };
    let t = Class(tables);
    let table = |key: &str| -> Option<TakedownTable> {
        tables.get(key)?;
        let f = |e: &Value, k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let u = |e: &Value, k: &str| e.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        let entries = t
            .field(key, "takedowns")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|e| TakedownEntry {
                        anim_name: e.get("anim_name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        lead: f(e, "f32_8"),
                        min: f(e, "f32_12"),
                        reach: f(e, "reach"),
                        angle_min: f(e, "angle_min"),
                        angle_max: f(e, "angle_max"),
                        side: u(e, "u32_28"),
                        min_speed: f32::from_bits(u(e, "u32_32")),
                        // Byte +36 of the big-endian word at +36.
                        mirrors: u(e, "u32_36") >> 24 != 0,
                        // Byte +38 bit 0: may trip on failure (`826A49D8`; b326).
                        trips: (u(e, "u32_36") >> 8) & 1 != 0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let d = TakedownTable::default();
        Some(TakedownTable { entries, reach_offset: t.f32(key, "Hash_8B45CA8123AD04D4").unwrap_or(d.reach_offset), slack: t.f32(key, "Hash_98DA4760B3C62ED7").unwrap_or(d.slack) })
    };
    let c = Class(entities);
    (entities.keys().filter_map(|e| Some((e.clone(), table(&c.reference(e, TAKEDOWN_FIELD)?)?))).collect(), table(ZOMBIE_KEY))
}

/// Which perceptions record an entity type uses (`livingworld_entities` field).
pub(crate) const PERCEPTION_FIELD: &str = "Hash_A27862C1107F64EC";

/// Each entity type's vision test ranges (`82E27240`): the mood event category
/// `chasedpersondetectibilitytest` near (`D16B`) / far (`A7EB`) times the perceptions record's
/// scales (`D036` / `D238`), its fov (`7435`), and the scales themselves (near, far, fov) for the
/// mood post's test with each category's ranges. Types without a record get none.
#[cfg(test)]
pub(crate) fn sight(bytes: &[u8]) -> (BTreeMap<String, Sight>, BTreeMap<String, [f32; 3]>) {
    let Ok(root) = serde_json::from_slice::<Value>(bytes) else { return Default::default() };
    sight_in(&root)
}

/// The same from the parsed `tables.json` (parsed once per load, shared by every reader).
pub(crate) fn sight_in(root: &Value) -> (BTreeMap<String, Sight>, BTreeMap<String, [f32; 3]>) {
    let classes = |n: &str| root.pointer(&format!("/classes/{n}")).and_then(Value::as_object).map(Class);
    let (Some(entities), Some(perceptions), Some(cats)) = (classes("livingworld_entities"), classes("livingworld_entities_perceptions"), classes("livingworld_moodeventcategories")) else {
        return Default::default();
    };
    let cat = "chasedpersondetectibilitytest";
    let (near, far) = (cats.f32(cat, "Hash_D16BF47CF5417C44").unwrap_or(0.0), cats.f32(cat, "Hash_A7EB37BC10C5A42F").unwrap_or(0.0));
    let scales: BTreeMap<String, [f32; 3]> = entities
        .0
        .keys()
        .filter_map(|e| {
            let p = entities.reference(e, PERCEPTION_FIELD)?;
            perceptions.0.get(&p)?;
            let f = |h: &str| perceptions.f32(&p, h).unwrap_or(0.0);
            Some((e.clone(), [f("Hash_D036FFCEEC9E7D03"), f("Hash_D238AC1B105361C6"), f("Hash_7435316F4A5E0B59")]))
        })
        .collect();
    (scales.iter().map(|(e, s)| (e.clone(), Sight::new(near, far, s[0], s[1], s[2]))).collect(), scales)
}

/// The conversation tables: every `livingworld_conversations` row, and per entity type the
/// categories of its conversation group (probability, row indices): entity field `66A6` -> the
/// conversation waypoint entity -> its group (`B4F9`) -> weighted categories -> rows.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConversationTables {
    pub rows: Vec<ConversationRow>,
    pub by_entity: BTreeMap<String, Vec<(f32, Vec<usize>)>>,
}

/// The sit plugin's values per entity type (`livingworld_entities`, parents merged): sit time min / max
/// (`Hash_1190326371F1A684` / `Hash_69F67C678B2673C9`) and the stand-up chance (`Hash_B040D387ABA6E24D`), read by
/// SetSitTimer `826A2898` and GoingToStandBackUp `826AD1F0`; a missing field keeps the retail default.
#[cfg(test)]
pub(crate) fn sit_values(bytes: &[u8]) -> BTreeMap<String, skate_core::living_world::peds::brain::SitValues> {
    let Ok(root) = serde_json::from_slice::<Value>(bytes) else { return Default::default() };
    sit_values_in(&root)
}

/// The same from the parsed `tables.json` (parsed once per load, shared by every reader).
pub(crate) fn sit_values_in(root: &Value) -> BTreeMap<String, skate_core::living_world::peds::brain::SitValues> {
    let Some(entities) = root.pointer("/classes/livingworld_entities").and_then(Value::as_object) else { return Default::default() };
    let e = Class(entities);
    let d = skate_core::living_world::peds::brain::SitValues::default();
    entities
        .keys()
        .map(|k| {
            let v = skate_core::living_world::peds::brain::SitValues {
                min_seconds: e.f32(k, "Hash_1190326371F1A684").unwrap_or(d.min_seconds),
                max_seconds: e.f32(k, "Hash_69F67C678B2673C9").unwrap_or(d.max_seconds),
                stand_up_chance: e.f32(k, "Hash_B040D387ABA6E24D").unwrap_or(d.stand_up_chance),
            };
            (k.clone(), v)
        })
        .collect()
}

/// Each entity type's starting hand prop: the carry chance (`Hash_3DB019A08284F45C`, read by `8269A588` in the ped
/// constructor `82E33198`) and its `handprop_odds` list of (handprop key, probability).
#[cfg(test)]
pub(crate) fn starting_hand_props(bytes: &[u8]) -> BTreeMap<String, (f32, Vec<(String, f32)>)> {
    let Ok(root) = serde_json::from_slice::<Value>(bytes) else { return Default::default() };
    starting_hand_props_in(&root)
}

/// The same from the parsed `tables.json` (parsed once per load, shared by every reader).
pub(crate) fn starting_hand_props_in(root: &Value) -> BTreeMap<String, (f32, Vec<(String, f32)>)> {
    let Some(entities) = root.pointer("/classes/livingworld_entities").and_then(Value::as_object) else { return Default::default() };
    let e = Class(entities);
    entities
        .iter()
        .map(|(k, v)| {
            let list = v.pointer("/fields/handprop_odds").and_then(Value::as_array).map_or_else(Vec::new, |a| {
                a.iter()
                    .filter_map(|x| Some((x.pointer("/ref/key")?.as_str()?.to_string(), x.get("probability")?.as_f64()? as f32)))
                    .collect()
            });
            (k.clone(), (e.f32(k, "Hash_3DB019A08284F45C").unwrap_or(0.0), list))
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn conversation_tables(bytes: &[u8]) -> ConversationTables {
    let Ok(root) = serde_json::from_slice::<Value>(bytes) else { return Default::default() };
    conversation_tables_in(&root)
}

/// The same from the parsed `tables.json` (parsed once per load, shared by every reader).
pub(crate) fn conversation_tables_in(root: &Value) -> ConversationTables {
    let class = |n: &str| root.pointer(&format!("/classes/{n}")).and_then(Value::as_object);
    let (Some(entities), Some(convs), Some(cats), Some(groups)) =
        (class("livingworld_entities"), class("livingworld_conversations"), class("livingworld_conversation_categories"), class("livingworld_conversation_category_groups"))
    else {
        return Default::default();
    };
    let (e, cv, ca, gr) = (Class(entities), Class(convs), Class(cats), Class(groups));
    let key = |v: &Value| v.get("key").and_then(Value::as_str).map(str::to_string);
    let mut t = ConversationTables::default();
    let mut index = BTreeMap::new();
    for k in convs.keys() {
        let participants = cv.field(k, "Hash_7819").and_then(Value::as_array).map(|a| a.iter().filter_map(key).collect()).unwrap_or_default();
        let values = cv.field(k, "Hash_36F1").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).map(|v| v as i32).collect()).unwrap_or_default();
        index.insert(k.clone(), t.rows.len());
        t.rows.push(ConversationRow { name: k.clone(), participants, values });
    }
    for k in entities.keys() {
        let Some(waypoint) = e.reference(k, "Hash_66A6") else { continue };
        let Some(group) = e.reference(&waypoint, "Hash_B4F9") else { continue };
        let list = gr.field(&group, "categories").and_then(Value::as_array).cloned().unwrap_or_default();
        let categories: Vec<(f32, Vec<usize>)> = list
            .iter()
            .filter_map(|c| {
                let cat = c.get("ref").and_then(key)?;
                let p = c.get("probability").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let rows = ca.field(&cat, "entities").and_then(Value::as_array).map(|a| a.iter().filter_map(key).filter_map(|r| index.get(&r).copied()).collect()).unwrap_or_default();
                Some((p, rows))
            })
            .collect();
        if !categories.is_empty() {
            t.by_entity.insert(k.clone(), categories);
        }
    }
    t
}

/// Entity type -> its reaction set name.
#[cfg(test)]
pub(crate) fn reaction_sets(bytes: &[u8]) -> BTreeMap<String, String> {
    let Ok(root) = serde_json::from_slice::<Value>(bytes) else { return BTreeMap::new() };
    reaction_sets_in(&root)
}

/// The same from the parsed `tables.json` (parsed once per load, shared by every reader).
pub(crate) fn reaction_sets_in(root: &Value) -> BTreeMap<String, String> {
    let Some(entities) = root.pointer("/classes/livingworld_entities").and_then(Value::as_object) else { return BTreeMap::new() };
    let c = Class(entities);
    entities.keys().filter_map(|k| Some((k.clone(), c.reference(k, REACTIONS_FIELD)?))).collect()
}

#[cfg(test)]
mod tests {
    /// Each entity type's starting prop chance and its (key, probability) list from tables.json.
    #[test]
    fn starting_hand_props_load_the_chance_and_the_list() {
        let json = br#"{"classes":{"livingworld_entities":{"granny":{"fields":{"Hash_3DB019A08284F45C":0.65,"handprop_odds":[{"ref":{"class":"livingworld_handprops","key":"purse"},"probability":0.3}]}},"chris_cole":{"fields":{"handprop_odds":[]}}}}}"#;
        let t = super::starting_hand_props(json);
        assert_eq!(t["granny"], (0.65, vec![("purse".to_string(), 0.3)]));
        assert_eq!(t["chris_cole"], (0.0, vec![]));
    }

    #[test]
    fn chase_records_merge_parents_per_entity_type() {
        let json = br#"{"classes": {
            "livingworld_entities": {"adult_male": {"parent": null, "fields": {"Hash_FBC42525E74D946F": {"class": "livingworld_entities_chase", "key": "pedestrians"}}}},
            "livingworld_entities_chase": {
                "default": {"parent": null, "fields": {"Hash_CD6575C0E03860E7": 5.0, "Hash_683215113C7201C2": false}},
                "global": {"parent": "default", "fields": {"Hash_5D480F3FFC02E98A": 22.5}},
                "pedestrians": {"parent": "default", "fields": {"escape_distance": 65.0, "Hash_CD6575C0E03860E7": 11.0}}
            }}}"#;
        let (by_entity, global, _) = super::chase_records(json);
        let r = &by_entity["adult_male"];
        assert_eq!((r.escape_distance(), r.run_speed()), (65.0, Some(11.0)));
        // Booleans are not numeric fields.
        assert_eq!(r.fields.len(), 2);
        assert_eq!(skate_core::living_world::peds::chase::predict_angle(global.as_ref()), 22.5);
    }

    /// The zombie cheat's records (b105): the group cap and the chaser fields come from `zombie`.
    #[test]
    fn zombie_chase_and_takedown_records_load_by_key() {
        let json = br#"{"classes": {
            "livingworld_entities": {},
            "livingworld_entities_chase": {
                "global": {"parent": null, "fields": {"Hash_CEA5D982FD05BE0B": 5}},
                "zombie": {"parent": null, "fields": {"Hash_CEA5D982FD05BE0B": 25, "escape_distance": 8.0, "give_up_after_takedowns": 1}}
            },
            "livingworld_entity_takedown": {"zombie": {"parent": null, "fields": {"takedowns": []}}}}}"#;
        let (_, global, zombie) = super::chase_records(json);
        use skate_core::living_world::peds::chase::ChaseRecord;
        assert_eq!((ChaseRecord::max_chasers(global.as_ref()), ChaseRecord::max_chasers(zombie.as_ref())), (5, 25));
        assert_eq!(zombie.unwrap().escape_distance(), 8.0);
        assert!(super::takedown_tables(json).1.is_some());
    }
}
