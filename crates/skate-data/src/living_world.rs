//! Readers for the living-world export the population core needs (census grid, tables, profiles, lines).
//! Formats: docs/hails-additions/living-world/peds-data.md, skaters-data.md; list: docs/hails-additions/26b-living-world-population.md#living-world-readers

use crate::aipath::AiPath;
use serde_json::Value;
use skate_core::living_world::census::CensusCategory;
use skate_core::living_world::npc_tricks::TrickProfile;
use skate_core::living_world::replay::{ReplayBranch, ReplayBranchGroup, ReplayJump, ReplayLine, ReplayNode};
use skate_core::living_world::{
    CensusCircle, CensusGrid, CensusMap, CensusRange, CensusRecord, PopulationConfig, SkaterCharacter, SkaterLine, VehicleCatalog, VehicleEntity,
};
use std::collections::BTreeMap;
use std::fmt;

pub const CENSUS_MAGIC: &[u8; 8] = b"LWCENSUS";
pub const CENSUS_VERSION: u32 = 1;
/// Range records the census kinds use (`livingworld_census_ranges`, census `+104` / `+112`).
pub const RANGE_PEDESTRIANS: &str = "pedestrians";
pub const RANGE_VEHICLES: &str = "vehicles";

#[derive(Debug, Clone, PartialEq)]
pub enum LivingWorldError {
    Census(String),
    Tables(String),
    Profiles(String),
}

impl fmt::Display for LivingWorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Census(m) => write!(f, "living-world census grid: {m}"),
            Self::Tables(m) => write!(f, "living-world tables: {m}"),
            Self::Profiles(m) => write!(f, "living-world skater profiles: {m}"),
        }
    }
}
impl std::error::Error for LivingWorldError {}

fn u32_at(d: &[u8], at: usize) -> Result<u32, LivingWorldError> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| LivingWorldError::Census(format!("truncated at {at}")))
}
fn f32_at(d: &[u8], at: usize) -> Result<f32, LivingWorldError> {
    u32_at(d, at).map(f32::from_bits)
}

/// Parse a `<District>.census.bin` grid.
pub fn parse_census_grid(data: &[u8]) -> Result<CensusGrid, LivingWorldError> {
    let err = |m: String| LivingWorldError::Census(m);
    if data.get(..8) != Some(&CENSUS_MAGIC[..]) {
        return Err(err("not a living-world census grid".into()));
    }
    let version = u32_at(data, 8)?;
    if version != CENSUS_VERSION {
        return Err(err(format!("unsupported version {version}")));
    }
    let cell = f32_at(data, 12)?;
    let origin = [f32_at(data, 16)?, f32_at(data, 20)?];
    let (width, height) = (u32_at(data, 24)?, u32_at(data, 28)?);
    let (layer_count, name_count) = (u32_at(data, 32)? as usize, u32_at(data, 36)? as usize);
    let cells = width as usize * height as usize;
    if !(cell > 0.0) || cells > 64 << 20 || layer_count > 64 {
        return Err(err(format!("bad header (cell {cell}, {width} x {height}, {layer_count} layers)")));
    }
    let mut cursor = 40;
    let mut table = Vec::new();
    for _ in 0..layer_count {
        let raw = data.get(cursor..cursor + 32).ok_or_else(|| err("truncated layer table".into()))?;
        let name = String::from_utf8_lossy(raw.split(|&b| b == 0).next().unwrap_or(&[])).into_owned();
        table.push((name, u32_at(data, cursor + 32)? as usize));
        cursor += 36;
    }
    let mut names = Vec::with_capacity(name_count);
    for _ in 0..name_count {
        let len = data.get(cursor..cursor + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(|| err("truncated names".into()))?;
        let raw = data.get(cursor + 2..cursor + 2 + len).ok_or_else(|| err("truncated name".into()))?;
        names.push(String::from_utf8(raw.to_vec()).map_err(|_| err("name is not text".into()))?);
        cursor += 2 + len;
    }
    let mut layers = BTreeMap::new();
    for (name, offset) in table {
        let raw = data.get(offset..offset + 2 * cells).ok_or_else(|| err(format!("layer {name} out of range")))?;
        let values: Vec<u16> = raw.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        if let Some(bad) = values.iter().find(|&&v| v as usize > names.len()) {
            return Err(err(format!("layer {name} names record {bad} of {}", names.len())));
        }
        layers.insert(name, values);
    }
    Ok(CensusGrid { cell, origin, width, height, names, layers })
}

/// The census part of `tables.json`, resolved.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LivingWorldTables {
    /// `livingworld_census` records by name: max population and the group's categories.
    pub census: BTreeMap<String, CensusRecord>,
    /// `livingworld_census_ranges` records by name.
    pub ranges: BTreeMap<String, CensusRange>,
}

fn circle(v: &Value) -> Option<CensusCircle> {
    let f = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| x as f32);
    Some(CensusCircle {
        spawn_inner: f("spawn_inner")?,
        spawn_outer: f("spawn_outer")?,
        cull: f("cull")?,
        forward_offset: f("forward_offset")?,
        speed_kmh: f("speed_kmh")?,
    })
}

impl LivingWorldTables {
    pub fn parse(json: &[u8]) -> Result<Self, LivingWorldError> {
        let doc: Value = serde_json::from_slice(json).map_err(|e| LivingWorldError::Tables(e.to_string()))?;
        Self::from_value(&doc)
    }

    pub fn from_value(doc: &Value) -> Result<Self, LivingWorldError> {
        let classes = doc.get("classes").ok_or_else(|| LivingWorldError::Tables("no classes".into()))?;
        let class = |name: &str| classes.get(name).and_then(Value::as_object);
        let groups = class("livingworld_categorygroups");
        let mut census = BTreeMap::new();
        for (key, row) in class("livingworld_census").into_iter().flatten() {
            let entry = &row["fields"]["entry"];
            let Some(max) = entry.get("max_population").and_then(Value::as_u64) else { continue };
            let group = entry["group"]["key"].as_str();
            let categories = group
                .and_then(|g| groups.and_then(|gs| gs.get(g)))
                .and_then(|g| g["fields"]["categories"].as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| Some(CensusCategory { name: i["category"]["key"].as_str()?.to_string(), weight: i["weight"].as_f64()? as f32 }))
                        .collect()
                })
                .unwrap_or_default();
            census.insert(key.clone(), CensusRecord { max_population: max as u32, categories });
        }
        let mut ranges = BTreeMap::new();
        for (key, row) in class("livingworld_census_ranges").into_iter().flatten() {
            let f = &row["fields"];
            if let (Some(slow), Some(fast)) = (circle(&f["circle_slow"]), circle(&f["circle_fast"])) {
                ranges.insert(key.clone(), CensusRange { slow, fast });
            }
        }
        if census.is_empty() || ranges.is_empty() {
            return Err(LivingWorldError::Tables("no census records or ranges".into()));
        }
        Ok(Self { census, ranges })
    }

    /// Fill the data side of a population config (ranges); code defaults stay.
    pub fn apply_to(&self, config: &mut PopulationConfig) {
        config.pedestrians.range = self.ranges.get(RANGE_PEDESTRIANS).copied();
        config.vehicles.range = self.ranges.get(RANGE_VEHICLES).copied();
    }

    /// A census map over the given district grids.
    pub fn census_map(&self, grids: Vec<CensusGrid>) -> CensusMap {
        CensusMap { grids, records: self.census.clone() }
    }
}

/// Free-roam pool characters from `skater_profiles.json`. Characters that need a recruited
/// teammate binding (`needs = recruited_save_slot`) are left out unless `bound` names them.
pub fn skater_characters(json: &[u8], bound: &[&str]) -> Result<Vec<SkaterCharacter>, LivingWorldError> {
    let doc: Value = serde_json::from_slice(json).map_err(|e| LivingWorldError::Profiles(e.to_string()))?;
    let pool = doc["free_roam_pool"].as_array().ok_or_else(|| LivingWorldError::Profiles("no free_roam_pool".into()))?;
    let chars = &doc["characters"];
    let mut out = Vec::new();
    for key in pool.iter().filter_map(Value::as_str) {
        let c = &chars[key];
        if c["needs"].as_str() == Some("recruited_save_slot") && !bound.contains(&key) {
            continue;
        }
        // The runtime AI profile's line capability bytes +0x31 / +0x30 / +0x32 (`sub_8245FFD8`, b675), against line
        // flag bits 0 / 1 / 2; a missing field reads false.
        let fields = &doc["ai_skater_profiles"][c["aiprofile"].as_str().unwrap_or(key)]["fields"];
        let capability = |h: &str| fields[h].as_bool().unwrap_or(false);
        out.push(SkaterCharacter {
            key: key.to_string(),
            pro_index: c["pro_index"].as_u64().map(|p| p as u32),
            capabilities: [capability("Hash_E01079D1A094B0C0"), capability("Hash_AACEBDA7CA734319"), capability("Hash_C36E34B4C619E868")],
            community: c["community"].as_bool().unwrap_or(false),
        });
    }
    Ok(out)
}

/// `ai_skater_profiles` field of the regular-stance table (flips + ollie; runtime profile +0,
/// layout +4, [code] `sub_8245FFD8`).
pub const PROFILE_REGULAR_TRICKS: &str = "Hash_E580B6284639E03F";
/// `ai_skater_profiles` field of the nollie table (nollie flips + nollie; profile +8, layout +164).
pub const PROFILE_NOLLIE_TRICKS: &str = "Hash_BB901D68361E9833";
/// `ai_skater_profiles` bool at layout +400 (copied to runtime profile +51 by `sub_8245FFD8`; layout offset from the
/// skaterschema): the recorded fingerflip / grab / boneless tricks take their slot (`sub_8246A2E0` 0x8246A61C).
/// Name unresolved; stock: true in 149 of 193 records.
pub const PROFILE_SLOT_TRICKS: &str = "Hash_6DBAF7AD6CDE6A16";

/// The AI skater profiles' trick tables, and which profile each character uses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkaterTrickProfiles {
    /// Trick tables by `ai_skater_profiles` record name (`default`, `street_medium`, ...), raw disc
    /// weights (the export resolves the records' inheritance).
    pub profiles: BTreeMap<String, TrickProfile>,
    /// `characters.*.aiprofile` by character key.
    pub character_profile: BTreeMap<String, String>,
    /// [`PROFILE_SLOT_TRICKS`] by `ai_skater_profiles` record name.
    pub slot_tricks: BTreeMap<String, bool>,
}

impl SkaterTrickProfiles {
    /// The table of a character: its `aiprofile` record, else `default`.
    pub fn for_character(&self, key: &str) -> Option<&TrickProfile> {
        self.character_profile.get(key).and_then(|p| self.profiles.get(p)).or_else(|| self.profiles.get("default"))
    }

    /// [`PROFILE_SLOT_TRICKS`] of a character's profile (else `default`'s; true when neither has it, the stock
    /// `default` value).
    pub fn slot_tricks_for(&self, key: &str) -> bool {
        let profile = self.character_profile.get(key).filter(|p| self.slot_tricks.contains_key(*p)).map_or("default", |p| p.as_str());
        self.slot_tricks.get(profile).copied().unwrap_or(true)
    }
}

/// One exported trick table (`[{trick, weight}]`): entries with a trick id in 0..332.
fn trick_table(v: &Value) -> Vec<(i16, f32)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let trick = i16::try_from(e["trick"].as_i64()?).ok().filter(|t| (0..332).contains(t))?;
            Some((trick, e["weight"].as_f64()? as f32))
        })
        .collect()
}

/// The AI profiles' trick tables and each character's profile from `skater_profiles.json`
/// (`ai_skater_profiles.*.fields`, `characters.*.aiprofile`).
pub fn skater_trick_profiles(json: &[u8]) -> Result<SkaterTrickProfiles, LivingWorldError> {
    let doc: Value = serde_json::from_slice(json).map_err(|e| LivingWorldError::Profiles(e.to_string()))?;
    let records = doc["ai_skater_profiles"].as_object().ok_or_else(|| LivingWorldError::Profiles("no ai_skater_profiles".into()))?;
    let profiles = records
        .iter()
        .map(|(name, r)| (name.clone(), TrickProfile { regular: trick_table(&r["fields"][PROFILE_REGULAR_TRICKS]), nollie: trick_table(&r["fields"][PROFILE_NOLLIE_TRICKS]) }))
        .collect();
    let character_profile = doc["characters"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, c)| c["aiprofile"].as_str().filter(|p| !p.is_empty()).map(|p| (k.clone(), p.to_owned())))
        .collect();
    let slot_tricks = records.iter().filter_map(|(name, r)| r["fields"][PROFILE_SLOT_TRICKS].as_bool().map(|b| (name.clone(), b))).collect();
    Ok(SkaterTrickProfiles { profiles, character_profile, slot_tricks })
}

/// Population view of decoded lines: ambient lines only, start = node 0, heading from node 0
/// towards the next node that is apart from it (the node orientation's component order is not
/// confirmed yet).
pub fn skater_lines<'a>(paths: impl IntoIterator<Item = &'a AiPath>) -> Vec<SkaterLine> {
    paths
        .into_iter()
        .filter(|p| p.id.is_ambient())
        .filter_map(|p| {
            let start = p.start()?;
            let next = p.nodes.iter().skip(1).map(|n| n.position).find(|q| (q[0] - start[0]).hypot(q[2] - start[2]) > 0.05);
            let heading = next.map_or(0.0, |q| (q[0] - start[0]).atan2(q[2] - start[2]));
            Some(SkaterLine { id: p.id.0, start, heading, valid: true, allowed_skaters: p.allowed_skaters, flags: p.flags })
        })
        .collect()
}

/// A decoded line as the replay cursor reads it (`skate_core::living_world::replay`): nodes,
/// jumps and branch groups unchanged (ids, frames, events, flags and orientations as on the disc).
pub fn replay_line(p: &AiPath) -> ReplayLine {
    ReplayLine {
        id: p.id.0,
        flags: p.flags,
        skill: p.skill_level,
        nodes: p
            .nodes
            .iter()
            .map(|n| ReplayNode {
                position: n.position,
                step: n.direction,
                board: n.board_orientation,
                skater: n.skater_orientation,
                frames: n.frames_since_last_node,
                event: n.event,
                flags: n.flags,
                jump: n.extended.map(|e| e as u32),
                width: [n.width_left, n.width_right],
            })
            .collect(),
        jumps: p
            .extended
            .iter()
            .map(|e| ReplayJump {
                start_position: e.trajectory_start_position,
                start_velocity: e.trajectory_start_velocity,
                offset: e.trajectory_offset,
                trick: e.trick_index,
                spins: e.air_spin_180_count,
                flags: e.flags,
            })
            .collect(),
        groups: p
            .branch_groups
            .iter()
            .map(|g| ReplayBranchGroup {
                node: g.node,
                branches: g.branches.iter().map(|b| ReplayBranch { target: b.target.0, target_node: b.target_node, weight: b.weight }).collect(),
            })
            .collect(),
    }
}

/// `vehicles.json` (milestone V0) -> the census [`VehicleCatalog`] (milestone V2): the entities of
/// every vehicle census category (`census.<record>.categories[].entities`, first listing wins)
/// and per entity its model, palette sizes (`palette_ids`) and size (`size_hint` x / z).
pub fn vehicle_catalog(json: &[u8]) -> Result<VehicleCatalog, LivingWorldError> {
    let doc: Value = serde_json::from_slice(json).map_err(|e| LivingWorldError::Tables(format!("vehicles.json: {e}")))?;
    let mut cat = VehicleCatalog::default();
    for record in doc["census"].as_object().into_iter().flat_map(|m| m.values()) {
        for c in record["categories"].as_array().into_iter().flatten() {
            let Some(name) = c["category"].as_str() else { continue };
            let list: Vec<String> = c["entities"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(str::to_string)).collect();
            cat.categories.entry(name.to_string()).or_insert(list);
        }
    }
    let models = &doc["models"];
    for list in cat.categories.values() {
        for e in list {
            let Some(model) = doc["entities"][e]["model"].as_str() else { continue };
            let m = &models[model];
            if m.is_null() {
                continue;
            }
            let count = |k: &str| m["palette_ids"][k].as_array().map_or(0, |a| a.len() as u32);
            let size = |k: &str| m["size_hint"][k].as_f64().unwrap_or(0.0) as f32;
            cat.entities.insert(
                e.clone(),
                VehicleEntity { model: model.to_string(), chassis_colours: count("chassis"), secondary_colours: count("secondary"), length: size("z"), width: size("x") },
            );
        }
    }
    Ok(cat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trick_profiles_read_both_tables_and_the_character_profile() {
        let json = br#"{"ai_skater_profiles": {
            "default": {"fields": {"Hash_E580B6284639E03F": [{"trick": 96, "weight": 1.0}, {"trick": 999, "weight": 1.0}],
                                   "Hash_BB901D68361E9833": [{"trick": 117, "weight": 0.5}]}},
            "danny_way": {"fields": {"Hash_E580B6284639E03F": [{"trick": 92, "weight": 0.1}]}}},
            "characters": {"danny_way": {"aiprofile": "danny_way"}, "jake": {"aiprofile": ""}}}"#;
        let t = skater_trick_profiles(json).unwrap();
        assert_eq!(t.profiles["default"], TrickProfile { regular: vec![(96, 1.0)], nollie: vec![(117, 0.5)] }, "ids outside 0..332 are dropped");
        assert_eq!(t.for_character("danny_way").unwrap().regular, [(92, 0.1)]);
        assert_eq!(t.for_character("jake"), t.profiles.get("default"), "no profile: default");
        assert!(skater_trick_profiles(b"{}").is_err());
    }

    /// A synthetic grid in the exporter's layout (mirrors `write_census_grid`).
    pub(crate) fn synth_grid(names: &[&str], layers: &[(&str, Vec<u16>)], w: u32, h: u32) -> Vec<u8> {
        let mut head = Vec::new();
        head.extend_from_slice(CENSUS_MAGIC);
        for v in [CENSUS_VERSION.to_le_bytes(), 4.0f32.to_le_bytes(), (-8.0f32).to_le_bytes(), 0.0f32.to_le_bytes(), w.to_le_bytes(), h.to_le_bytes(), (layers.len() as u32).to_le_bytes(), (names.len() as u32).to_le_bytes()] {
            head.extend_from_slice(&v);
        }
        let mut name_bytes = Vec::new();
        for n in names {
            name_bytes.extend_from_slice(&(n.len() as u16).to_le_bytes());
            name_bytes.extend_from_slice(n.as_bytes());
        }
        let header = 40 + 36 * layers.len();
        let mut cursor = header + name_bytes.len();
        cursor += (4 - cursor % 4) % 4;
        let mut table = Vec::new();
        let mut body = Vec::new();
        for (name, cells) in layers {
            let mut n = name.as_bytes().to_vec();
            n.resize(32, 0);
            table.extend_from_slice(&n);
            table.extend_from_slice(&((cursor + body.len()) as u32).to_le_bytes());
            for c in cells {
                body.extend_from_slice(&c.to_le_bytes());
            }
            while body.len() % 4 != 0 {
                body.push(0);
            }
        }
        let mut blob = [head, table, name_bytes].concat();
        while blob.len() % 4 != 0 {
            blob.push(0);
        }
        blob.extend_from_slice(&body);
        blob
    }

    #[test]
    fn census_grid_round_trip_and_lookup() {
        // 3 x 2 cells of 4 m from (-8, 0): x in [-8, 4), z in [0, 8).
        let data = synth_grid(&["aletown", "dwntwn"], &[("livingworld_npc_census", vec![0, 1, 1, 1, 0, 0]), ("livingworld_vehicle_census", vec![2; 6])], 3, 2);
        let g = parse_census_grid(&data).unwrap();
        assert_eq!((g.width, g.height, g.cell, g.origin), (3, 2, 4.0, [-8.0, 0.0]));
        assert_eq!(g.record_at("livingworld_npc_census", -7.0, 1.0), None);
        assert_eq!(g.record_at("livingworld_npc_census", -3.0, 1.0), Some("aletown"));
        assert_eq!(g.record_at("livingworld_npc_census", -7.0, 5.0), Some("aletown"));
        assert_eq!(g.record_at("livingworld_npc_census", 1.0, 5.0), None);
        assert_eq!(g.record_at("livingworld_vehicle_census", 3.9, 7.9), Some("dwntwn"));
        assert_eq!(g.record_at("livingworld_vehicle_census", 4.0, 1.0), None);
        assert!(parse_census_grid(&data[..50]).is_err());
        assert!(parse_census_grid(b"NOTCENSUS0000000").is_err());
    }

    #[test]
    fn tables_resolve_census_records_and_ranges() {
        let json = br#"{"version":1,"classes":{
            "livingworld_census":{"aletown":{"parent":"downtown","fields":{"entry":{"group":{"class":"livingworld_categorygroups","key":"aletown"},"max_population":15}}},
                                  "broken":{"parent":null,"fields":{}}},
            "livingworld_categorygroups":{"aletown":{"parent":null,"fields":{"categories":[{"category":{"class":"livingworld_entitycategories","key":"aletown"},"weight":1.0}]}}},
            "livingworld_census_ranges":{"pedestrians":{"parent":null,"fields":{
                "circle_slow":{"spawn_inner":50.0,"spawn_outer":60.0,"cull":70.0,"forward_offset":0.0,"speed_kmh":45.0},
                "circle_fast":{"spawn_inner":50.0,"spawn_outer":80.0,"cull":90.0,"forward_offset":20.0,"speed_kmh":80.0}}}}}}"#;
        let t = LivingWorldTables::parse(json).unwrap();
        assert_eq!(t.census["aletown"].max_population, 15);
        assert_eq!(t.census["aletown"].categories[0].name, "aletown");
        assert!(!t.census.contains_key("broken"));
        let mut cfg = PopulationConfig::retail();
        t.apply_to(&mut cfg);
        assert_eq!(cfg.pedestrians.range.unwrap().slow.cull, 70.0);
        assert!(cfg.vehicles.range.is_none());
    }
}
