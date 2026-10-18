//! Ped plugin data (doc 26g "Ped plugins on world props"): the prop classes (`livingworld_props`: descriptor, hand
//! props, numeric fields kept by hash until decoded), their descriptors (`PluginDescriptor/<name>.stategraph`: graph,
//! max participants, transfer expression), each ped type's `plugin_odds` (`livingworld_entities`, parents merged) and
//! the district's placed plugin props (`waypoints.json`, from the RW waypoint groups). The prop side and the rolls are
//! `skate_core::living_world::peds::plugins`. DMO hotpoint seats (benches) need the hotpoint export (open).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use skate_core::living_world::peds::plugins::{Descriptor, PluginProp, PluginWaypoint, Transfer};

/// One prop class (`livingworld_props.<class>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PluginClass {
    pub descriptor: Option<Descriptor>,
    /// Hand props a ped may take there (`Hash_E15E856F2CA9B96B`: {handprop, probability}).
    pub hand_props: Vec<(String, f32)>,
    /// The record's other numeric fields by hash (`Hash_E2101F2B17A0E6B5` = 10 / 5 / 15 is a radius candidate, b82;
    /// meanings open).
    pub numbers: BTreeMap<String, f32>,
}

/// Everything the ped plugin runtime reads.
#[derive(Clone, Debug, Default)]
pub(crate) struct PluginData {
    pub classes: BTreeMap<String, PluginClass>,
    pub odds: BTreeMap<String, Vec<(String, f32)>>,
    /// Placed plugin props by district name.
    pub placed: BTreeMap<String, Vec<PluginProp>>,
}

/// `plugin_odds` per entity type, the nearest record in the parent chain that has it.
fn odds(entities: &serde_json::Map<String, Value>) -> BTreeMap<String, Vec<(String, f32)>> {
    let find = |key: &str| {
        let mut k = Some(key.to_string());
        for _ in 0..32 {
            let rec = entities.get(k.as_deref()?)?;
            if let Some(list) = rec.pointer("/fields/plugin_odds").and_then(Value::as_array) {
                return Some(list.iter().filter_map(|e| Some((e.pointer("/ref/key")?.as_str()?.to_string(), e.get("probability")?.as_f64()? as f32))).collect());
            }
            k = rec.get("parent").and_then(Value::as_str).map(str::to_string);
        }
        None
    };
    entities.keys().filter_map(|k| Some((k.clone(), find(k)?))).collect()
}

/// A descriptor from its compiled `.stategraph` (the `BehaviourPlugin` element and its `TransferExpression`).
pub(crate) fn descriptor(root: &Path, xml_path: &str) -> Result<Descriptor, String> {
    let name = xml_path.rsplit(['\\', '/']).next().unwrap_or(xml_path).trim_end_matches(".xml").to_string();
    let file = root.join("private/stock/data/livingworld/PluginDescriptor").join(format!("{name}.stategraph"));
    let g = skate_data::state_graph::StateGraph::load(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let attr = |i: usize, k: &str| g.elements[i].attributes.iter().find(|a| a.name.eq_ignore_ascii_case(k));
    let plugin = (0..g.elements.len()).find(|&i| g.elements[i].tag == "BehaviourPlugin").ok_or("no BehaviourPlugin")?;
    fn tree(g: &skate_data::state_graph::StateGraph, i: usize) -> Option<Transfer> {
        let e = &g.elements[i];
        match e.tag.as_str() {
            "TransferCondition" => Some(Transfer::Condition {
                name: e.attributes.iter().find(|a| a.name == "name").map(|a| a.text.clone()).unwrap_or_default(),
                params: e.attributes.iter().filter(|a| a.name != "name").map(|a| (a.name.clone(), a.text.clone())).collect(),
            }),
            "TransferExpression" => {
                let children: Vec<Transfer> = e.children.iter().filter_map(|&c| tree(g, c)).collect();
                Some(match e.attributes.iter().find(|a| a.name == "op").map(|a| a.text.to_ascii_lowercase()).as_deref() {
                    Some("or") => Transfer::Or(children),
                    Some("not") => Transfer::Not(children),
                    _ => Transfer::And(children),
                })
            }
            _ => None,
        }
    }
    let transfer = g.elements[plugin].children.iter().find_map(|&c| tree(&g, c));
    let graph = attr(plugin, "file").map(|a| a.text.clone()).unwrap_or_default();
    let max_participants = attr(plugin, "maxNumberOfParticipants").and_then(|a| a.text.parse::<f32>().ok()).unwrap_or(1.0).max(0.0) as u32;
    Ok(Descriptor { name, graph, max_participants, transfer })
}

/// The district's placed waypoint groups as plugin props (`waypoints.json`).
fn placed(waypoints: &Value) -> BTreeMap<String, Vec<PluginProp>> {
    let v3 = |v: &Value| -> Option<[f32; 3]> {
        let a = v.as_array()?;
        Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
    };
    let mut out = BTreeMap::new();
    let Some(districts) = waypoints.get("districts").and_then(Value::as_object) else { return out };
    for (district, groups) in districts {
        let props = groups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| {
                let id = u64::from_str_radix(g.get("id")?.as_str()?, 16).ok()?;
                let waypoints = g.get("waypoints")?.as_array()?.iter().filter_map(|w| Some(PluginWaypoint { position: v3(w.get("position")?)?, facing: v3(w.get("facing")?)?, occupant: None })).collect();
                Some(PluginProp { id, class: g.get("type")?.as_str()?.to_string(), waypoints, cooldown: 0.0, cooldown_reset: 0.0, disabled: false })
            })
            .collect();
        out.insert(district.clone(), props);
    }
    out
}

/// The map's DMO hotpoint plugin props (`native-props/<map>.json` `plugin_props`, setup `hotpoint_data`): one
/// one-waypoint prop per hotpoint of each placed bench, bin or newspaper box (`82C4E128`), at the initial placement.
/// The id is the DMO instance id with the hotpoint index in its top byte (stable for a host). The hotpoints belong to
/// the object, so a moved prop carries its seats (`follow_prop`, applied every think from the body's pose).
/// A hotpoint authored at `position` / `facing` on a prop authored at `spawn` (origin, basis columns), carried to the
/// prop's pose `now`: the point keeps its place on the object (a bench pushed aside takes its seats along; peds used
/// to sit in the air where it had been, play test 2026-10-12).
pub(crate) fn follow_prop(position: [f32; 3], facing: [f32; 3], spawn: ([f32; 3], [[f32; 3]; 3]), now: ([f32; 3], [[f32; 3]; 3])) -> ([f32; 3], [f32; 3]) {
    // Local = spawn basis transposed (orthonormal) times the offset; world = basis times local.
    let to_local = |v: [f32; 3]| std::array::from_fn::<f32, 3, _>(|i| (0..3).map(|k| spawn.1[i][k] * v[k]).sum());
    let to_world = |l: [f32; 3]| std::array::from_fn::<f32, 3, _>(|k| (0..3).map(|i| now.1[i][k] * l[i]).sum());
    let local = to_local(std::array::from_fn(|k| position[k] - spawn.0[k]));
    let p = to_world(local);
    (std::array::from_fn(|k| p[k] + now.0[k]), to_world(to_local(facing)))
}

pub(crate) fn map_props(root: &Path, map: &str) -> Vec<PluginProp> {
    let path = root.join("private/native-props").join(format!("{map}.json"));
    // skate_vita: through `skate_data::vfs` (props.pak).
    let Some(report) = skate_data::vfs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return Vec::new() };
    let v3 = |v: &Value| -> Option<[f32; 3]> {
        let a = v.as_array()?;
        Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
    };
    report
        .get("plugin_props")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let instance = u64::from_str_radix(h.get("instance_id")?.as_str()?, 16).ok()?;
            let index = h.get("index")?.as_u64()?;
            Some(PluginProp {
                id: instance ^ ((index + 1) << 56),
                class: h.get("class")?.as_str()?.to_string(),
                waypoints: vec![PluginWaypoint { position: v3(h.get("position")?)?, facing: v3(h.get("facing")?)?, occupant: None }],
                cooldown: 0.0,
                cooldown_reset: 0.0,
                disabled: false,
            })
        })
        .collect()
}

impl PluginData {
    /// Load from the asset root (`private/living_world/tables.json` and `waypoints.json`, the stock descriptors).
    pub fn load(root: &Path) -> Result<Self, String> {
        // skate_vita: through `skate_data::vfs`; the packs carry the peds' classes in `ped_tables.json`.
        let dir = root.join("private/living_world");
        let bytes = skate_data::vfs::read(&dir.join("ped_tables.json")).or_else(|_| skate_data::vfs::read(&dir.join("tables.json"))).map_err(|e| e.to_string())?;
        let tables: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let entities = tables.pointer("/classes/livingworld_entities").and_then(Value::as_object).ok_or("tables.json has no livingworld_entities")?;
        let props = tables.pointer("/classes/livingworld_props").and_then(Value::as_object).ok_or("tables.json has no livingworld_props")?;
        let field = |key: &str, name: &str| -> Option<Value> {
            let mut k = Some(key.to_string());
            for _ in 0..32 {
                let rec = props.get(k.as_deref()?)?;
                if let Some(v) = rec.get("fields").and_then(|f| f.get(name)) {
                    return Some(v.clone());
                }
                k = rec.get("parent").and_then(Value::as_str).map(str::to_string);
            }
            None
        };
        let mut classes = BTreeMap::new();
        for key in props.keys() {
            let path = field(key, "Hash_48BD5E5A1C5EE7A1").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            // The disc ships gatheringplace_01..04 (and their gatherbehaviourgeneric graph) only as .xml source, no
            // compiled .stategraph [data]; every other descriptor ships both. That class gets no plugin here (retail
            // most likely the same [inferred]: its loader reading the .xml is not decoded).
            let descriptor = if path.is_empty() { None } else { descriptor(root, &path).map_err(|e| info!("PED_PLUGINS {key}: no compiled descriptor, class has no plugin: {e}")).ok() };
            // A one-entry list is exported as the bare entry (waypoint_newspaperbox), not an array.
            let hand_props = field(key, "Hash_E15E856F2CA9B96B")
                .map(|v| match v {
                    Value::Array(a) => a,
                    Value::Object(_) => vec![v],
                    _ => Vec::new(),
                })
                .unwrap_or_default()
                .iter()
                .filter_map(|e| Some((e.pointer("/ref/key")?.as_str()?.to_string(), e.get("probability")?.as_f64()? as f32)))
                .collect();
            let mut numbers = BTreeMap::new();
            let mut k = Some(key.clone());
            for _ in 0..32 {
                let Some(rec) = k.as_deref().and_then(|x| props.get(x)) else { break };
                for (n, v) in rec.get("fields").and_then(Value::as_object).into_iter().flatten() {
                    if let Some(x) = v.as_f64() {
                        numbers.entry(n.clone()).or_insert(x as f32);
                    }
                }
                k = rec.get("parent").and_then(Value::as_str).map(str::to_string);
            }
            classes.insert(key.clone(), PluginClass { descriptor, hand_props, numbers });
        }
        let waypoints = skate_data::vfs::read(&root.join("private/living_world/waypoints.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        Ok(Self { classes, odds: odds(entities), placed: waypoints.as_ref().map(placed).unwrap_or_default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odds_and_placed_groups_parse() {
        let entities = serde_json::json!({
            "regular": {"parent": null, "fields": {}},
            "adult_male": {"parent": "regular", "fields": {"plugin_odds": [{"ref": {"class": "livingworld_entities", "key": "waypoint_sit"}, "probability": 0.8}]}},
            "adult_male01": {"parent": "adult_male", "fields": {}}
        });
        let o = odds(entities.as_object().unwrap());
        assert_eq!(o["adult_male01"], vec![("waypoint_sit".to_string(), 0.8)]);
        assert!(!o.contains_key("regular"));
        let w = serde_json::json!({"version": 1, "districts": {"DownTown": [{"id": "99FA4D88899F3731", "type": "waypoint_vendingmachine", "waypoints": [{"position": [1.0, 2.0, 3.0], "facing": [-1.0, 0.0, 0.0]}]}]}});
        let p = placed(&w);
        assert_eq!(p["DownTown"][0].id, 0x99FA4D88899F3731);
        assert_eq!(p["DownTown"][0].class, "waypoint_vendingmachine");
        assert_eq!(p["DownTown"][0].waypoints[0].position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn hotpoint_props_load_with_stable_ids() {
        let dir = std::env::temp_dir().join(format!("skate-plugin-props-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("private/native-props")).unwrap();
        std::fs::write(
            dir.join("private/native-props/Test.json"),
            r#"{"plugin_props": [{"instance_id": "BBDDDCD5E794B3CE", "template_id": "T", "index": 1, "type": 6, "class": "waypoint_sit", "position": [10.25, 1.0, 19.5], "facing": [1.0, 0.0, 0.0]}]}"#,
        )
        .unwrap();
        let props = map_props(&dir, "Test");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].id, 0xBBDDDCD5E794B3CE ^ (2 << 56));
        assert_eq!(props[0].class, "waypoint_sit");
        assert_eq!(props[0].waypoints[0].position, [10.25, 1.0, 19.5]);
        assert!(map_props(std::path::Path::new("Z:/nowhere"), "Test").is_empty());
    }

    #[test]
    fn a_moved_prop_carries_its_seats() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        // A seat 1 m in front (+z) of a bench at (10, 0, 20), facing back towards it.
        let spawn = ([10.0, 0.0, 20.0], identity);
        assert_eq!(follow_prop([10.0, 0.5, 21.0], [0.0, 0.0, -1.0], spawn, spawn), ([10.0, 0.5, 21.0], [0.0, 0.0, -1.0]));
        // Pushed 3 m along x and turned 90 degrees about y (+z goes to +x).
        let turned = [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
        let (p, f) = follow_prop([10.0, 0.5, 21.0], [0.0, 0.0, -1.0], spawn, ([13.0, 0.0, 20.0], turned));
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(near(p, [14.0, 0.5, 20.0]), "{p:?}");
        assert!(near(f, [-1.0, 0.0, 0.0]), "{f:?}");
    }

    /// The stock descriptors parse: sit's transfer tree and the conversation's 3 participants.
    #[test]
    #[ignore = "requires the private stock data (SKATE3_ASSET_ROOT=<install>/assets)"]
    fn stock_descriptors_parse() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let data = PluginData::load(&root).unwrap();
        let sit = data.classes["waypoint_sit"].descriptor.as_ref().expect("sit descriptor");
        assert_eq!(sit.max_participants, 1);
        let mut names = Vec::new();
        sit.transfer.as_ref().unwrap().names(&mut names);
        eprintln!("sit graph {} transfer {names:?}", sit.graph);
        assert!(names.contains(&"HasFirstWaypointAvailable".to_string()) && names.contains(&"IsChasing".to_string()));
        assert_eq!(data.classes["waypoint_conversation"].descriptor.as_ref().map(|d| d.max_participants), Some(3));
        // Hand props per class: a list (vending machine) and a bare one-entry record (newspaper box).
        assert_eq!(data.classes["waypoint_vendingmachine"].hand_props, vec![("pop".to_string(), 0.5), ("waterbottle".to_string(), 0.5)]);
        assert_eq!(data.classes["waypoint_newspaperbox"].hand_props, vec![("newspaper".to_string(), 1.0)]);
        assert!(data.odds["adult_female01"].iter().any(|(c, p)| c == "waypoint_sit" && (*p - 0.7).abs() < 1e-6));
        eprintln!("placed: {:?}", data.placed.iter().map(|(k, v)| (k.clone(), v.len())).collect::<Vec<_>>());
        assert!(data.placed.values().map(Vec::len).sum::<usize>() > 0);
    }
}
