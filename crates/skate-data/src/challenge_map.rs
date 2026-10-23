//! The retail Challenge Map tree (FE screen 17) from the vault collections, as menu map nodes.
//! Data rules (b244, b255, b447, b450): docs/hails-additions/31-retail-menus.md#challenge-map-tree
use crate::attrib_hash::{hash, numeric_name};
use crate::collections::Collection;
use skate_core::menus::{EntryKind, EntryRef, MapNode};
use std::collections::BTreeMap;

const LISTING_FILTER: &str = "Hash_E0762EA3F90C9EEB";
const LISTING_ORDER: &str = "Hash_982BD1EAE2334504";
const FILTER_TITLE: &str = "Hash_6766EFD84D74F4F0";
const LOCATION_LABEL: &str = "Hash_174B301910A902C9";
const LOCATION_DISTRICT: &str = "Hash_50D9293B818507EC";

/// Which `world` each location filter lists (retail's filter Lua, as data; the DLC folders come with the packs' rows,
/// research b447). Mods may change it.
#[derive(Clone, Debug, PartialEq)]
pub struct LocationRules(pub BTreeMap<String, String>);

impl Default for LocationRules {
    fn default() -> Self {
        let pairs = [
            ("downtown", "dist_downtown"),
            ("industrial", "dist_industrial"),
            ("university", "dist_university"),
            ("skate_school", "dist_skateschool"),
            ("maloof", "dist_maloofmoneycup"),
            ("blackbox_park", "dist_blackboxpark"),
            ("maloofmoneycupdlc", "dist_maloofmoneycupdlc"),
            ("dwmegacompounddlc", "dlc_dw_megacompound"),
            ("sanitariumdlc", "dist_sanitariumdlc"),
            ("artgallerydlc", "dist_artgallerydlc"),
        ];
        Self(pairs.into_iter().map(|(a, b)| (a.to_owned(), b.to_owned())).collect())
    }
}

/// A location row's "always shown" flag and name text (retail `Unlock.IsLocationUnlocked`, `82839C38`, research b450).
const LOCATION_SHOWN: &str = "Hash_90E678B6961730E1";
const LOCATION_NAME: &str = "Hash_942AB8AEE4B414ED";

/// Which locations count as unlocked (b450 / b458; NOT RETAIL YET until the unlock store exists): docs/hails-additions/31-retail-menus.md#location-unlocks
#[derive(Clone, Debug, PartialEq)]
pub struct LocationUnlocks {
    /// Retail's `all_locations_unlock` (cheat id 2, which no TU3 code triggers) and the manager's unlock-all byte.
    pub all: bool,
    pub granted: std::collections::BTreeSet<String>,
    pub locked: std::collections::BTreeSet<String>,
}

impl Default for LocationUnlocks {
    fn default() -> Self {
        Self { all: false, granted: Default::default(), locked: ["gamestart", "dist_skateschool"].map(str::to_owned).into() }
    }
}

impl LocationUnlocks {
    pub fn unlocked(&self, name: &str) -> bool {
        self.all || self.granted.contains(name) || !self.locked.contains(name)
    }

    /// The `LOCATIONS` lines of setup's `private/dlc-unlocks.json` (the player's DLC unlock files, b458); a fresh
    /// save's view when the file is missing.
    pub fn load(asset_root: &std::path::Path) -> Self {
        let path = asset_root.join("private/dlc-unlocks.json");
        let Ok(bytes) = std::fs::read(&path) else { return Self::default() };
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let granted = value["unlocks"].as_array().into_iter().flatten()
            .filter(|u| u["kind"].as_str() == Some("LOCATIONS"))
            .filter_map(|u| u["name"].as_str().map(str::to_owned))
            .collect();
        Self { granted, ..Self::default() }
    }
}

/// Known `map_listing` names, so the tabs' hook ids read as words (other rows keep their vault key, `Hash_...`).
const KNOWN_LISTINGS: [&str; 10] = [
    "progression", "progression_locations", "progression_active", "progression_completed", "progression_all",
    "online", "online_competitive", "online_team", "freeskate", "location",
];

/// `map_listing` field `IsDebug` (research b479: retail's tab and folder filter 82D0C0E0 skips a row with it set).
const LISTING_IS_DEBUG: &str = "Hash_755772463A459824";

/// The `map_listing` ids flagged `IsDebug` (TU3: only `progression_all`, the "debug_list_all" tab), named as `load`
/// names them.
pub fn debug_listings(entries: &[Collection]) -> std::collections::BTreeSet<String> {
    let names: BTreeMap<u64, &str> = KNOWN_LISTINGS.iter().map(|n| (hash(n), *n)).collect();
    entries
        .iter()
        .filter(|e| class_is(e, "map_listing"))
        .filter(|e| text(e, LISTING_IS_DEBUG).is_some_and(|d| d.trim_start_matches('0').len() > 0))
        .map(|e| names.get(&key_hash(&e.key)).map_or_else(|| e.key.clone(), |n| (*n).to_owned()))
        .collect()
}

fn class_is(entry: &Collection, class: &str) -> bool {
    numeric_name(&entry.class_name) == numeric_name(class)
}

fn text<'a>(entry: &'a Collection, field: &str) -> Option<&'a str> {
    entry.fields.get(field).map(|f| f.data.as_str())
}

/// A RefSpec field's (class hash, key hash) from its hex (`CCCCCCCCCCCCCCCC KKKKKKKKKKKKKKKK`).
fn ref_spec(entry: &Collection, field: &str) -> Option<(u64, u64)> {
    let data = text(entry, field)?;
    let class = u64::from_str_radix(data.get(0..16)?, 16).ok()?;
    let key = u64::from_str_radix(data.get(16..32)?, 16).ok()?;
    Some((class, key))
}

/// A typed RefSpec field's key (`ssRefSpec_<class>`: the class is the type, the data is `KKKKKKKKKKKKKKKK` + padding).
fn typed_ref_key(entry: &Collection, field: &str) -> Option<u64> {
    u64::from_str_radix(text(entry, field)?.get(0..16)?, 16).ok()
}

const WORLD_CLASS: &str = "world";

/// Each location's map picture name (FE `82649FF8`: the location's district names its `world` record, whose
/// tMapInfo names the picture), by location key. `world_maps` = the world records that set their own name (the menu
/// set's `world_maps`, resolved at export, b259); a record without one inherits its parent's.
pub fn location_maps(entries: &[Collection], world_maps: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let worlds: Vec<&Collection> = entries.iter().filter(|e| class_is(e, WORLD_CLASS)).collect();
    let by_hash: BTreeMap<u64, &str> = worlds.iter().copied().map(|e| (key_hash(&e.key), e.key.as_str())).collect();
    let parent: BTreeMap<&str, &str> = worlds.iter().copied().map(|e| (e.key.as_str(), e.parent.as_str())).collect();
    fn name_of<'a>(mut world: &'a str, parent: &BTreeMap<&'a str, &'a str>, maps: &BTreeMap<String, String>) -> Option<String> {
        for _ in 0..16 {
            if let Some(name) = maps.get(world) {
                return Some(name.clone());
            }
            world = parent.get(world)?;
        }
        None
    }
    entries
        .iter()
        .filter(|e| class_is(e, "fe_locations"))
        .filter_map(|loc| {
            // NOT RETAIL YET (b417): the DLC locations' District names a map_category (Maloof NYC), not a world, so
            // their World field picks the record; which field retail reads there is unverified.
            let world = [LOCATION_DISTRICT, LOCATION_WORLD].iter().find_map(|f| by_hash.get(&typed_ref_key(loc, f)?))?;
            Some((loc.key.clone(), name_of(world, &parent, world_maps)?))
        })
        .filter(|(_, name)| !name.is_empty())
        .collect()
}

const LOCATION_ICON: &str = "Hash_786255B8FF426A3D";
const LOCATION_WORLD: &str = "Hash_7565CD2A80E93A5E";
const ICON_CLASS: &str = "Hash_F788504A8D922CED";
const ICON_NAME: &str = "Hash_FCFE787CCF2F775E";
const WORLD_OFFSET: &str = "Hash_B88A4034D63E905E";
const WORLD_MAP_INFO: &str = "Hash_20BBC594CD2FAEF8";

/// What a location's Challenge Map marker needs besides its locator (research b264 / b265).
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerSource {
    /// The icon texture name: the location's `Icon` (fe_map_icons RefSpec, inherited) record's name text.
    pub icon: String,
    /// The location's `World` record's offset (Vector2, inherited; added to the locator's x / z).
    pub offset: [f32; 2],
    /// That world's tMapInfo b1..b6 (inherited): the projection bounds and map size.
    pub bounds: [i32; 6],
}

/// The marker data per fe_locations key; a location whose World or icon cannot be read is left out.
pub fn location_markers(collections: &crate::collections::Collections) -> BTreeMap<String, MarkerSource> {
    let entries = collections.entries();
    let icons: BTreeMap<u64, &str> =
        entries.iter().filter(|e| class_is(e, ICON_CLASS)).map(|e| (key_hash(&e.key), e.key.as_str())).collect();
    let worlds: BTreeMap<u64, &str> =
        entries.iter().filter(|e| class_is(e, WORLD_CLASS)).map(|e| (key_hash(&e.key), e.key.as_str())).collect();
    let hex = |data: &str, at: usize| data.get(at..at + 8).and_then(|h| u32::from_str_radix(h, 16).ok());
    entries
        .iter()
        .filter(|e| class_is(e, "fe_locations"))
        .filter_map(|loc| {
            let field = |name: &str| collections.field("fe_locations", &loc.key, name).ok().map(|f| f.data.as_str());
            // Icon: Attrib::RefSpec = class, key; World: ClassRefSpec = key first.
            let icon_key = u64::from_str_radix(field(LOCATION_ICON)?.get(16..32)?, 16).ok()?;
            let world_key = u64::from_str_radix(field(LOCATION_WORLD)?.get(0..16)?, 16).ok()?;
            let icon = collections.field(ICON_CLASS, icons.get(&icon_key)?, ICON_NAME).ok()?.data.as_str().to_owned();
            let world = worlds.get(&world_key)?;
            let offset = collections.field(WORLD_CLASS, world, WORLD_OFFSET).ok().map_or([0.0; 2], |f| {
                [0, 8].map(|at| hex(&f.data, at).map_or(0.0, f32::from_bits))
            });
            let info = collections.field(WORLD_CLASS, world, WORLD_MAP_INFO).ok()?;
            let mut bounds = [0i32; 6];
            for (i, b) in bounds.iter_mut().enumerate() {
                *b = hex(&info.data, 8 + 8 * i)? as i32;
            }
            Some((loc.key.clone(), MarkerSource { icon, offset, bounds }))
        })
        .collect()
}

/// A `Hash_XXXXXXXXXXXXXXXX` key, or `hash(name)` for a readable one.
fn key_hash(key: &str) -> u64 {
    key.strip_prefix("Hash_").and_then(|h| u64::from_str_radix(h, 16).ok()).unwrap_or_else(|| hash(key))
}

/// The skate.Park folder's filter and the `world` fields it reads (research b447).
const SKATEPARK_FILTER: &str = "skatepark";
const PARK_FLAG: &str = "IsSkatePark";
const PARK_TAB_FLAG: &str = "AllowInSkateParksTab";
/// A world row's own display text id (`ID_WORLD_SKATEPARK_MEGAPARK_FULL`).
const WORLD_LABEL: &str = "Hash_095E1D9778F0C6F4";

/// A leaf's category class and the category's sort order (Int32; research b451).
const CATEGORY_CLASS: &str = "map_category";
const CATEGORY_ORDER: &str = "Hash_1349BB6EC1D1E751";
/// A category header's label id (`82644430`, b454) and the folder flag that drops a lone category's header.
const CATEGORY_LABEL: &str = "Hash_465698DD3CC73CA2";
const FOLDER_LONE_HEADERLESS: &str = "Hash_EAFE7F250C154184";

/// The label collation order (b451: `8290F0A8` ranks each character by its position in the language's sort string;
/// upper and lower case interleaved, then the digits). Ours, written to that rule; NOT RETAIL YET: the language's own
/// string (likely `ID_COMMON_SORTING_FORMAT`, with accented letters) is not exported, so a mod language cannot set it.
pub const COLLATION: &str = "AaBbCcDdEeFfGgHhIiJjKkLlMmNnOoPpQqRrSsTtUuVvWwXxYyZz0123456789";

/// `8290F0A8`'s comparison: character by character by position in `order`, a character missing from it after every
/// listed one; a string that is a prefix of the other compares equal.
pub fn collate(a: &str, b: &str, order: &str) -> std::cmp::Ordering {
    let rank = |c: char| order.chars().position(|o| o == c).unwrap_or(usize::MAX);
    a.chars().zip(b.chars()).map(|(x, y)| rank(x).cmp(&rank(y))).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
}

/// Field lookup through a row's parents in its class (the vault's inheritance), over a plain row list.
struct Inherited<'a> {
    rows: BTreeMap<(u64, u64), &'a Collection>,
}

impl<'a> Inherited<'a> {
    fn new(entries: &'a [Collection]) -> Self {
        Self { rows: entries.iter().map(|e| ((key_hash(&numeric_name(&e.class_name)), key_hash(&e.key)), e)).collect() }
    }

    fn get(&self, row: &'a Collection, field: &str) -> Option<&'a str> {
        let wanted = numeric_name(field);
        let class = key_hash(&numeric_name(&row.class_name));
        let mut at = row;
        for _ in 0..16 {
            if let Some(f) = at.fields.get(field).or_else(|| at.fields.get(&wanted)) {
                return Some(f.data.as_str());
            }
            if at.parent.is_empty() {
                return None;
            }
            at = self.rows.get(&(class, key_hash(&at.parent)))?;
        }
        None
    }
}
/// The tree under `root` as map nodes, folders in retail's sort order (b451).
pub fn load(entries: &[Collection], root: &str, rules: &LocationRules, unlocks: &LocationUnlocks,
    label_text: &dyn Fn(&str) -> Option<String>) -> Vec<MapNode> {
    let categories: BTreeMap<u64, &Collection> =
        entries.iter().filter(|e| class_is(e, CATEGORY_CLASS)).map(|e| (key_hash(&e.key), e)).collect();
    let names: BTreeMap<u64, &str> = KNOWN_LISTINGS.iter().map(|n| (hash(n), *n)).collect();
    let id_of = |key: &str| names.get(&key_hash(key)).map_or_else(|| key.to_owned(), |n| (*n).to_owned());
    let filters: BTreeMap<u64, &Collection> = entries.iter().filter(|e| class_is(e, "map_filter")).map(|e| (key_hash(&e.key), e)).collect();
    let listings: Vec<&Collection> = entries.iter().filter(|e| class_is(e, "map_listing")).collect();
    let locations: Vec<&Collection> = entries.iter().filter(|e| class_is(e, "fe_locations")).collect();
    let worlds: Vec<&Collection> = entries.iter().filter(|e| class_is(e, WORLD_CLASS)).collect();
    let inherited = Inherited::new(entries);
    // A leaf's category (`Hash_50D9293B818507EC`, a map_category ref, inherited; `82644228`) and that category's order.
    let category_of = |row: &Collection| -> Option<&Collection> {
        inherited.get(row, LOCATION_DISTRICT).and_then(|d| u64::from_str_radix(d.get(0..16)?, 16).ok())
            .and_then(|key| categories.get(&key)).copied()
    };
    let category_order = |c: Option<&Collection>| -> i32 {
        c.and_then(|c| inherited.get(c, CATEGORY_ORDER)).and_then(|d| u32::from_str_radix(d.get(0..8)?, 16).ok()).map_or(0, |v| v as i32)
    };
    let root_entry = EntryRef::map(EntryKind::MapListing, root);
    let mut nodes = vec![MapNode { entry: root_entry.clone(), parent: None, label: root.to_owned(), order: 0 }];
    let mut queue = vec![(key_hash(root), root_entry)];
    while let Some((parent_hash, parent)) = queue.pop() {
        for row in listings.iter().filter(|r| key_hash(&r.parent) == parent_hash) {
            let entry = EntryRef::map(EntryKind::MapListing, &id_of(&row.key));
            let order = text(row, LISTING_ORDER).and_then(|d| d.get(0..4)).and_then(|d| i32::from_str_radix(d, 16).ok()).unwrap_or(0);
            let folder = ref_spec(row, LISTING_FILTER);
            let filter = folder.filter(|(class, _)| *class == hash("map_filter")).and_then(|(_, key)| filters.get(&key));
            // A folder may point at a map_category instead (the debug tree's `progression_all_team_spacer`): its label
            // then comes from the category, empty for a spacer row.
            let category = folder.filter(|(class, _)| *class == hash("map_category")).and_then(|(_, key)| categories.get(&key));
            let label = filter.and_then(|f| text(f, FILTER_TITLE))
                .or_else(|| category.map(|c| text(c, CATEGORY_LABEL).unwrap_or("")))
                .unwrap_or(&entry.id).to_owned();
            nodes.push(MapNode { entry: entry.clone(), parent: Some(parent.clone()), label, order });
            // A folder's leaves: (category order, category, label, leaf).
            let mut leaves: Vec<(i32, Option<&Collection>, String, MapNode)> = Vec::new();
            let mut leaf = |row: &Collection, kind: EntryKind, label: &str| {
                let node = MapNode { entry: EntryRef::map(kind, &row.key), parent: Some(entry.clone()), label: label.to_owned(), order: 0 };
                let category = category_of(row);
                leaves.push((category_order(category), category, label.to_owned(), node));
            };
            // The location leaves of a location filter.
            if let Some(world) = filter.and_then(|f| rules.0.get(&f.key)) {
                let world = hash(world);
                for loc in &locations {
                    let in_world = inherited.get(loc, LOCATION_WORLD).and_then(|d| u64::from_str_radix(d.get(0..16)?, 16).ok()) == Some(world);
                    let shown = inherited.get(loc, LOCATION_SHOWN).is_some_and(|d| d.trim_start_matches('0') != "")
                        || unlocks.unlocked(inherited.get(loc, LOCATION_NAME).unwrap_or(&loc.key));
                    if in_world && shown {
                        leaf(loc, EntryKind::FeLocation, text(loc, LOCATION_LABEL).unwrap_or(&loc.key));
                    }
                }
            }
            // The skate.Park folder: its park worlds (b447).
            if filter.is_some_and(|f| f.key == SKATEPARK_FILTER) {
                let on = |w: &Collection, field: &str| inherited.get(w, field).is_some_and(|d| d.starts_with("01"));
                for world in worlds.iter().filter(|w| on(w, PARK_FLAG) && on(w, PARK_TAB_FLAG)) {
                    leaf(world, EntryKind::World, inherited.get(world, WORLD_LABEL).unwrap_or(&world.key));
                }
            }
            // Retail folder sort (82647EA0, comparator 82647B80, b451): category order, then label text; ours stable.
            let display = |label: &str| label_text(label).unwrap_or_else(|| label.to_owned());
            leaves.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| collate(&display(&a.2), &display(&b.2), COLLATION)));
            // A map_category header row before each category run (b454); uncategorised leaves get none (NOT RETAIL YET).
            let mut kinds: Vec<&str> = leaves.iter().filter_map(|l| l.1.map(|c| c.key.as_str())).collect();
            kinds.dedup();
            let lone = inherited.get(row, FOLDER_LONE_HEADERLESS).is_some_and(|d| d.trim_start_matches('0') != "") && kinds.len() == 1;
            let mut previous: Option<&str> = None;
            let mut rank = 0;
            for (_, category, _, mut node) in leaves {
                if let Some(c) = category.filter(|c| !lone && previous != Some(c.key.as_str())) {
                    let label = inherited.get(c, CATEGORY_LABEL).unwrap_or(&c.key).to_owned();
                    nodes.push(MapNode { entry: EntryRef::map(EntryKind::MapCategory, &c.key), parent: Some(entry.clone()), label, order: rank });
                    rank += 1;
                }
                previous = category.map(|c| c.key.as_str());
                node.order = rank;
                rank += 1;
                nodes.push(node);
            }
            queue.push((key_hash(&row.key), entry));
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::Field;

    fn field(data: &str) -> Field {
        Field { type_name: "".into(), data: data.into(), array: None }
    }
    fn row(class: &str, key: &str, parent: &str, fields: &[(&str, String)]) -> Collection {
        Collection {
            class_name: numeric_name(class),
            key: key.to_owned(),
            parent: parent.to_owned(),
            fields: fields.iter().map(|(k, v)| (k.to_string(), field(v))).collect(),
            source: String::new(),
            sha256: String::new(),
        }
    }
    fn refspec(class: &str, key: &str) -> String {
        format!("{:016X}{:016X}", hash(class), hash(key))
    }
    /// A typed RefSpec (`ssRefSpec_map_category`): the key, then padding.
    fn typed(key: &str) -> String {
        format!("{:016X}0000000000000000", hash(key))
    }

    #[test]
    fn a_location_shows_its_districts_world_map_inherited_up_the_world_records() {
        let rows = [
            row("world", "default", "", &[]),
            row("world", "default_skate3", "default", &[]),
            row("world", "dist_skateschool", "default_skate3", &[]),
            row("world", "dist_university", "default_skate3", &[]),
            row("fe_locations", "school_spot", "skate3", &[(LOCATION_DISTRICT, typed("dist_skateschool"))]),
            row("fe_locations", "uni_spot", "skate3", &[(LOCATION_DISTRICT, typed("dist_university"))]),
            row("fe_locations", "nowhere", "skate3", &[]),
        ];
        let named = |pairs: &[(&str, &str)]| pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<BTreeMap<_, _>>();
        let worlds = named(&[("default", "map1024"), ("default_skate3", "map_portcarverton"), ("dist_skateschool", "map_skateschool")]);
        let maps = location_maps(&rows, &worlds);
        assert_eq!(maps, named(&[("school_spot", "map_skateschool"), ("uni_spot", "map_portcarverton")]));
        assert!(location_maps(&rows, &BTreeMap::new()).is_empty(), "an older menu set names no maps");
    }

    #[test]
    fn a_dlc_location_whose_district_is_no_world_takes_its_world_field() {
        let rows = [
            row("world", "default", "", &[]),
            row("world", "dist_maloofmoneycupdlc", "default", &[]),
            row("fe_locations", "dist_maloofmoneycupdlc", "skate3", &[(LOCATION_DISTRICT, typed("maloofmoneycupdlc")),
                (LOCATION_WORLD, typed("dist_maloofmoneycupdlc"))]),
        ];
        let worlds: BTreeMap<String, String> = [("default".to_string(), "map1024".to_string()),
            ("dist_maloofmoneycupdlc".to_string(), "DLC_Maloof_Map_512".to_string())].into();
        assert_eq!(location_maps(&rows, &worlds).get("dist_maloofmoneycupdlc").map(String::as_str), Some("DLC_Maloof_Map_512"));
    }

    #[test]
    fn the_locations_tab_lists_each_districts_locations_in_its_folder() {
        let tab = format!("Hash_{:016X}", hash("progression_locations"));
        let folder = "Hash_00000000000000AA";
        let entries = vec![
            row("map_listing", &tab, "progression", &[(LISTING_FILTER, refspec("map_filter", "location")), (LISTING_ORDER, "00030000".into())]),
            row("map_listing", folder, &tab, &[(LISTING_FILTER, refspec("map_filter", "downtown")), (LISTING_ORDER, "00010000".into())]),
            row("map_filter", "downtown", "location", &[(FILTER_TITLE, "ID_DISTRICT_DOWNTOWN".into())]),
            // Retail lists by World (b447), not District: a District naming another place does not matter.
            row("fe_locations", "dist_downtown_aletown", "skate3", &[(LOCATION_LABEL, "ID_LOT_DT_01C".into()),
                (LOCATION_DISTRICT, typed("dist_maloof")), (LOCATION_WORLD, typed("dist_downtown")), (LOCATION_SHOWN, "01000000".into())]),
            row("fe_locations", "dist_university_x", "skate3", &[(LOCATION_WORLD, typed("dist_university")), (LOCATION_SHOWN, "01000000".into())]),
        ];
        let nodes = load(&entries, "progression", &LocationRules::default(), &LocationUnlocks::default(), &|_| None);
        let ids: Vec<String> = nodes.iter().map(|n| n.entry.hook_id()).collect();
        assert_eq!(ids, ["map_listing:progression", "map_listing:progression_locations", "map_listing:Hash_00000000000000AA", "fe_locations:dist_downtown_aletown"]);
        let aletown = nodes.last().unwrap();
        assert_eq!((aletown.parent.as_ref().map(|p| p.id.as_str()), aletown.label.as_str()), (Some(folder), "ID_LOT_DT_01C"));
        assert_eq!((nodes[2].label.as_str(), nodes[2].order, nodes[1].order), ("ID_DISTRICT_DOWNTOWN", 1, 3));
        // A mod rule moves the university location under the folder instead.
        let mut rules = LocationRules::default();
        rules.0.insert("downtown".into(), "dist_university".into());
        assert_eq!(load(&entries, "progression", &rules, &LocationUnlocks::default(), &|_| None).last().unwrap().entry.id, "dist_university_x");
    }

    #[test]
    fn the_dlc_folders_list_by_world_and_the_skate_park_folder_lists_park_worlds() {
        let tab = format!("Hash_{:016X}", hash("progression_locations"));
        let entries = vec![
            row("map_listing", &tab, "progression", &[(LISTING_FILTER, refspec("map_filter", "location"))]),
            row("map_listing", "Hash_00000000000000AA", &tab, &[(LISTING_FILTER, refspec("map_filter", "maloofmoneycupdlc"))]),
            row("map_listing", "Hash_00000000000000BB", &tab, &[(LISTING_FILTER, refspec("map_filter", "skatepark"))]),
            row("map_filter", "maloofmoneycupdlc", "location", &[]),
            row("map_filter", "skatepark", "location", &[]),
            row("fe_locations", "skate3", "", &[]),
            row("fe_locations", "dist_maloofmoneycupdlc", "skate3", &[(LOCATION_WORLD, typed("dist_maloofmoneycupdlc")), (LOCATION_SHOWN, "01000000".into())]),
            row("world", "default_skateparks", "", &[("IsSkatePark", "01".into())]),
            row("world", "dist_megapark_full", "default_skateparks", &[(WORLD_LABEL, "ID_WORLD_SKATEPARK_MEGAPARK_FULL".into()),
                ("AllowInSkateParksTab", "01".into())]),
            row("world", "dist_megapark", "default_skateparks", &[]),
            row("world", "dist_downtown", "", &[]),
        ];
        let nodes = load(&entries, "progression", &LocationRules::default(), &LocationUnlocks::default(), &|_| None);
        let under = |folder: &str| -> Vec<String> {
            nodes.iter().filter(|n| n.parent.as_ref().is_some_and(|p| p.id == folder)).map(|n| n.entry.hook_id()).collect()
        };
        assert_eq!(under("Hash_00000000000000AA"), ["fe_locations:dist_maloofmoneycupdlc"], "a DLC folder lists its pack's location by World");
        assert_eq!(under("Hash_00000000000000BB"), ["world:dist_megapark_full"], "both flags, inherited; the Empty row is off the tab");
        let park = nodes.iter().find(|n| n.entry.id == "dist_megapark_full").unwrap();
        assert_eq!(park.label, "ID_WORLD_SKATEPARK_MEGAPARK_FULL");
    }

    #[test]
    fn a_folder_sorts_by_category_order_then_label_text() {
        let tab = format!("Hash_{:016X}", hash("progression_locations"));
        let entries = vec![
            row("map_listing", &tab, "progression", &[(LISTING_FILTER, refspec("map_filter", "location"))]),
            row("map_listing", "Hash_00000000000000BB", &tab, &[(LISTING_FILTER, refspec("map_filter", "skatepark"))]),
            row("map_filter", "skatepark", "location", &[]),
            row("map_category", "late", "", &[(CATEGORY_ORDER, "00000258".into())]),
            row("map_category", "early", "", &[(CATEGORY_ORDER, "00000064".into())]),
            row("world", "parks", "", &[("IsSkatePark", "01".into())]),
            row("world", "z_first_in_data", "parks", &[(WORLD_LABEL, "ID_ZEN".into()), (LOCATION_DISTRICT, typed("late")), ("AllowInSkateParksTab", "01".into())]),
            row("world", "b_full", "parks", &[(WORLD_LABEL, "ID_B_FULL".into()), (LOCATION_DISTRICT, typed("early")), ("AllowInSkateParksTab", "01".into())]),
            row("world", "b_basic", "parks", &[(WORLD_LABEL, "ID_B_BASIC".into()), (LOCATION_DISTRICT, typed("early")), ("AllowInSkateParksTab", "01".into())]),
        ];
        let texts = |id: &str| match id { "ID_B_FULL" => Some("PCU Training - Full".to_owned()),
            "ID_B_BASIC" => Some("PCU Training - Basic".to_owned()), _ => None };
        let nodes = load(&entries, "progression", &LocationRules::default(), &LocationUnlocks::default(), &texts);
        let parks: Vec<(&str, i32)> = nodes.iter().filter(|n| n.entry.kind == EntryKind::World).map(|n| (n.entry.id.as_str(), n.order)).collect();
        assert_eq!(parks, [("b_basic", 1), ("b_full", 2), ("z_first_in_data", 4)], "category 100 before 600, then Basic before Full");
        // b454: a header row before each category run.
        let folder: Vec<(EntryKind, &str)> = nodes.iter().filter(|n| n.parent.as_ref().is_some_and(|p| p.id == "Hash_00000000000000BB"))
            .map(|n| (n.entry.kind, n.entry.id.as_str())).collect();
        assert_eq!(folder, [(EntryKind::MapCategory, "early"), (EntryKind::World, "b_basic"), (EntryKind::World, "b_full"),
            (EntryKind::MapCategory, "late"), (EntryKind::World, "z_first_in_data")]);
        // The folder flag drops the header only when the folder holds one category.
        let mut flagged = entries.clone();
        flagged[1].fields.insert(FOLDER_LONE_HEADERLESS.into(), field("01"));
        let headers = |e: &[Collection]| load(e, "progression", &LocationRules::default(), &LocationUnlocks::default(), &texts)
            .iter().filter(|n| n.entry.kind == EntryKind::MapCategory).count();
        assert_eq!(headers(&flagged), 2, "two categories keep their headers");
        flagged.retain(|r| r.key != "z_first_in_data");
        assert_eq!(headers(&flagged), 0, "a lone category under the flag has none");
        assert_eq!(collate("a", "B", COLLATION), std::cmp::Ordering::Less, "a before B: case interleaved");
        assert_eq!(collate("A", "a", COLLATION), std::cmp::Ordering::Less);
        assert_eq!(collate("ab", "abc", COLLATION), std::cmp::Ordering::Equal, "a prefix compares equal");
        assert_eq!(collate("z", "~", COLLATION), std::cmp::Ordering::Less, "unlisted characters sort last");
    }

    #[test]
    fn a_location_without_the_shown_flag_needs_its_unlock() {
        let tab = format!("Hash_{:016X}", hash("progression_locations"));
        let entries = vec![
            row("map_listing", &tab, "progression", &[(LISTING_FILTER, refspec("map_filter", "location"))]),
            row("map_listing", "Hash_00000000000000AA", &tab, &[(LISTING_FILTER, refspec("map_filter", "university"))]),
            row("map_filter", "university", "location", &[]),
            row("fe_locations", "uni", "", &[(LOCATION_WORLD, typed("dist_university")), (LOCATION_SHOWN, "01000000".into())]),
            row("fe_locations", "dist_university_library", "uni", &[(LOCATION_NAME, "dist_university_library".into())]),
            // b450: `gamestart` clears its parent's flag, so its name must be unlocked.
            row("fe_locations", "gamestart", "dist_university_library", &[(LOCATION_NAME, "gamestart".into()), (LOCATION_SHOWN, "00000000".into())]),
            row("fe_locations", "late_spot", "uni", &[(LOCATION_NAME, "late_spot".into()), (LOCATION_SHOWN, "00000000".into())]),
        ];
        let leaves = |unlocks: &LocationUnlocks| -> Vec<String> {
            load(&entries, "progression", &LocationRules::default(), unlocks, &|_| None).iter()
                .filter(|n| n.entry.kind == EntryKind::FeLocation).map(|n| n.entry.id.clone()).collect()
        };
        assert_eq!(leaves(&LocationUnlocks::default()), ["dist_university_library", "late_spot", "uni"], "inherited flag; gamestart locked");
        let mut unlocks = LocationUnlocks::default();
        unlocks.locked.insert("late_spot".into());
        assert_eq!(leaves(&unlocks), ["dist_university_library", "uni"], "a locked name without a grant is hidden");
        unlocks.granted.insert("late_spot".into());
        assert_eq!(leaves(&unlocks).len(), 3, "a grant (DLC line, mod) beats the lock");
        unlocks.all = true;
        assert_eq!(leaves(&unlocks).len(), 4, "all_locations_unlock");
        // setup's dlc-unlocks.json: only LOCATIONS lines grant.
        let root = std::env::temp_dir().join(format!("skate-unlocks-{}", std::process::id()));
        std::fs::create_dir_all(root.join("private")).unwrap();
        std::fs::write(root.join("private/dlc-unlocks.json"), r#"{"version":1,"unlocks":[
            {"source":"Time is Money","file":"a.unlock","kind":"LOCATIONS","name":"late_spot"},
            {"source":"x","file":"b.unlock","kind":"CHARACTERS","name":"gamestart"}]}"#).unwrap();
        let loaded = LocationUnlocks::load(&root);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(loaded.granted, ["late_spot".to_owned()].into());
        assert_eq!(LocationUnlocks::load(std::path::Path::new("missing")), LocationUnlocks::default());
    }
    /// The real tree: four tabs, the seven location folders in retail order, Downtown's 14 locations.
    #[test]
    #[ignore = "requires the converted vault (SKATE3_ASSET_ROOT with private/stock/skater-collections.json)"]
    fn the_retail_progression_tree() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let collections = crate::collections::Collections::load(&root).unwrap();
        let data = skate_core::menus::MenuData { map_nodes: load(collections.entries(), "progression", &LocationRules::default(), &LocationUnlocks::default(), &|_| None), ..Default::default() };
        let root_entry = EntryRef::map(EntryKind::MapListing, "progression");
        let tabs: Vec<&str> = data.map_children(&root_entry).iter().map(|n| n.entry.id.as_str()).collect();
        assert_eq!(tabs, ["progression_active", "progression_completed", "progression_locations", "progression_all"]);
        let folders = data.map_children(&EntryRef::map(EntryKind::MapListing, "progression_locations"));
        let labels: Vec<&str> = folders.iter().map(|n| n.label.as_str()).collect();
        // The disc's seven folders, then one per DLC pack installed (b447: Maloof NYC, Hawaiian Dream, The Sanatorium,
        // Art Gallery with the user's packs).
        assert!(labels.len() >= 7, "{labels:?}");
        assert_eq!(labels[0], "ID_DISTRICT_DOWNTOWN");
        let downtown = data.map_children(&folders[0].entry);
        assert_eq!(downtown.len(), 14);
        // b450 / b458: University lists 10 (gamestart's flag is 0 and no package grants it), skate.School 1 (no
        // dist_skateschool), as in the retail video.
        assert_eq!(data.map_children(&folders[2].entry).len(), 10);
        assert_eq!(data.map_children(&folders[3].entry).len(), 1);
        // skate.Park (folder 4): the park worlds with both tab flags; 5 on the disc (+3 with the Create / San Van packs).
        // b454: a map_category header before each park's category run (8 parks + 7 headers with the packs).
        let parks = data.map_children(&folders[4].entry);
        let worlds = parks.iter().filter(|n| n.entry.kind == EntryKind::World).count();
        let headers = parks.iter().filter(|n| n.entry.kind == EntryKind::MapCategory).count();
        eprintln!("SKATE_PARK rows {:?}", parks.iter().map(|n| n.label.as_str()).collect::<Vec<_>>());
        assert!(worlds >= 5 && worlds + headers == parks.len() && parks[0].entry.kind == EntryKind::MapCategory, "{parks:?}");
        assert!(parks.iter().any(|n| n.entry.id == "dist_megapark_full"));
        // Every DLC folder lists its pack's location.
        assert!(folders[7..].iter().all(|f| !data.map_children(&f.entry).is_empty()), "{labels:?}");
        assert!(downtown.iter().all(|n| n.entry.kind == EntryKind::FeLocation));
        assert!(downtown.iter().any(|n| n.entry.id == "dist_downtown_aletown"));
        // Markers (b264 / b265): every career location's icon is mini_teleport; Downtown's world offset and bounds.
        let markers = location_markers(&collections);
        let aletown = &markers["dist_downtown_aletown"];
        assert_eq!(aletown.icon, "mini_teleport");
        assert_eq!(aletown.offset, [225.0, -175.0]);
        assert_eq!(aletown.bounds, [2050, -850, 1050, -1250, 512, 512]);
        assert!(downtown.iter().all(|n| markers.get(&n.entry.id).is_some_and(|m| m.icon == "mini_teleport")));
    }
}

/// skate_vita: the Challenge Map as the Vita reads it (vita-bake writes it into ui.pak: the device does not load the
/// collections a second time): the tree, each location's picture and marker (skate-game `retail_menus::challenge_map`)
/// and the teleport of each location that has one (`private/teleports.json`: its map and locator matrix).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Baked {
    pub nodes: Vec<MapNode>,
    pub pictures: BTreeMap<String, String>,
    pub markers: BTreeMap<String, skate_core::menus::MapMarker>,
    pub teleports: BTreeMap<String, (String, [[f32; 4]; 4])>,
    /// The listings retail shows only with Debug Menus on (`debug_listings`, upstream `74c124a`).
    pub debug: std::collections::BTreeSet<String>,
}

/// skate-game `retail_menus::challenge_map` over the collections, the set's world maps and `teleports.json`.
/// `unlocks`: [`LocationUnlocks::load`] of the installation. The leaves sort by their label's English text, which
/// `teleports.json` carries per destination (`label_key`, `name`), as upstream's `retail_menus::challenge_map`.
pub fn bake(collections: &crate::collections::Collections, world_maps: &BTreeMap<String, String>, teleports: &serde_json::Value,
    unlocks: &LocationUnlocks) -> Baked {
    let texts: BTreeMap<String, String> = teleports["destinations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| Some((d["label_key"].as_str()?.to_owned(), d["name"].as_str()?.to_owned())))
        .collect();
    let label_text = |label: &str| texts.get(label).cloned();
    let teleports: BTreeMap<String, (String, [[f32; 4]; 4])> = teleports["destinations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let matrix: [[f32; 4]; 4] = serde_json::from_value(d["matrix"].clone()).ok()?;
            Some((d["id"].as_str()?.to_owned(), (d["map"].as_str()?.to_owned(), matrix)))
        })
        .collect();
    let markers = location_markers(collections)
        .into_iter()
        .filter_map(|(key, m)| {
            let (_, matrix) = teleports.get(&key)?;
            let point = [matrix[3][0] + m.offset[0], matrix[3][2] + m.offset[1]];
            Some((key, skate_core::menus::MapMarker { icon: m.icon, point, bounds: m.bounds }))
        })
        .collect();
    Baked {
        nodes: load(collections.entries(), "progression", &LocationRules::default(), unlocks, &label_text),
        pictures: location_maps(collections.entries(), world_maps),
        markers,
        teleports,
        debug: debug_listings(collections.entries()),
    }
}

impl Baked {
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::json;
        let entry = |e: &EntryRef| json!({"kind": e.kind.name(), "id": e.id});
        json!({
            "nodes": self.nodes.iter().map(|n| json!({
                "entry": entry(&n.entry), "parent": n.parent.as_ref().map(entry), "label": n.label, "order": n.order,
            })).collect::<Vec<_>>(),
            "pictures": self.pictures,
            "markers": self.markers.iter().map(|(k, m)| (k.clone(), json!({"icon": m.icon, "point": m.point, "bounds": m.bounds}))).collect::<serde_json::Map<_, _>>(),
            "teleports": self.teleports.iter().map(|(k, (map, matrix))| (k.clone(), json!({"map": map, "matrix": matrix}))).collect::<serde_json::Map<_, _>>(),
            "debug": self.debug,
        })
    }

    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let bad = |what: &str| format!("challenge map JSON: bad {what}");
        let entry = |v: &serde_json::Value| -> Result<EntryRef, String> {
            let kind = EntryKind::parse(v["kind"].as_str().unwrap_or_default()).ok_or_else(|| bad("entry kind"))?;
            Ok(EntryRef { kind, id: v["id"].as_str().ok_or_else(|| bad("entry id"))?.to_owned() })
        };
        let mut baked = Baked::default();
        for n in v["nodes"].as_array().ok_or_else(|| bad("nodes"))? {
            let parent = if n["parent"].is_null() { None } else { Some(entry(&n["parent"])?) };
            baked.nodes.push(MapNode { entry: entry(&n["entry"])?, parent, label: n["label"].as_str().unwrap_or_default().to_owned(), order: n["order"].as_i64().unwrap_or(0) as i32 });
        }
        baked.pictures = serde_json::from_value(v["pictures"].clone()).map_err(|_| bad("pictures"))?;
        for (k, m) in v["markers"].as_object().ok_or_else(|| bad("markers"))? {
            let point = serde_json::from_value(m["point"].clone()).map_err(|_| bad("marker point"))?;
            let bounds = serde_json::from_value(m["bounds"].clone()).map_err(|_| bad("marker bounds"))?;
            baked.markers.insert(k.clone(), skate_core::menus::MapMarker { icon: m["icon"].as_str().unwrap_or_default().to_owned(), point, bounds });
        }
        for (k, t) in v["teleports"].as_object().ok_or_else(|| bad("teleports"))? {
            let matrix = serde_json::from_value(t["matrix"].clone()).map_err(|_| bad("teleport matrix"))?;
            baked.teleports.insert(k.clone(), (t["map"].as_str().unwrap_or_default().to_owned(), matrix));
        }
        // Older bakes have none.
        baked.debug = serde_json::from_value(v["debug"].clone()).unwrap_or_default();
        Ok(baked)
    }
}

#[cfg(test)]
mod baked_tests {
    use super::*;

    /// The installed data (`SKATE3_ASSETS=<install>/assets`): the map bakes and survives its JSON form.
    #[test]
    #[ignore = "diagnostic; requires the installed data"]
    fn the_installed_map_bakes_and_reads_back() {
        let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSETS").unwrap());
        let collections = crate::collections::Collections::load(&assets).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(assets.join("private/menu-movies/manifest.json")).unwrap()).unwrap();
        let world_maps: BTreeMap<String, String> = serde_json::from_value(manifest["world_maps"].clone()).unwrap();
        let teleports: serde_json::Value = serde_json::from_slice(&std::fs::read(assets.join("private/teleports.json")).unwrap()).unwrap();
        let baked = bake(&collections, &world_maps, &teleports, &LocationUnlocks::default());
        let json = baked.to_json();
        eprintln!(
            "MAP {} nodes, {} pictures, {} markers, {} teleports, {} KiB",
            baked.nodes.len(), baked.pictures.len(), baked.markers.len(), baked.teleports.len(), json.to_string().len() / 1024
        );
        assert_eq!(Baked::from_json(&json).unwrap(), baked);
        assert!(!baked.nodes.is_empty() && !baked.teleports.is_empty());
    }
}
