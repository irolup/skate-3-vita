//! The front-end menu tables (`assets/private/menu-tables.json`): skate-3-rust-engine's
//! `skate_data::menu_tables` (PR #52 `fad5aae`), the data side only. The tables come from the
//! user's own executable: `tools/menu-tables` writes them with the reference checkout's extractor,
//! which unpacks the executable; this repo holds no part of that (README, "Making your own game
//! data"). Copied: the records, the JSON form and `to_menu_data`; not copied: `extract`,
//! `json_from_xex` and the table locators (the `xex` and `sha` modules they use).
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsRow {
    /// Language string id (`ID_GAMESETTINGS_*`); retail leaves a few rows blank.
    pub id: String,
    /// `option` (a link or action), `slider` or `selector`.
    pub widget: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    pub label: String,
    pub icon: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub name: String,
    pub label: String,
    pub icon: String,
    /// Sub-option list kind (a second column), -1 = none.
    pub sub_option_kind: i32,
    pub help: String,
}

/// A front-end screen template (retail table 0x8302F1A8 in TU3, 16-byte records {name, swf path, type, -1}):
/// the screen manager registers them in table order, so a screen's AS id is 600 + its index.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenTemplate {
    pub name: String,
    /// Path under `source/screens/` (`main/core_menu.swf`); empty for the "Ignore" record.
    pub path: String,
    pub kind: u32,
}

/// One FE screen stack record. Numeric ids differ between builds (TU3 inserts two), so `id` is the identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeScreen {
    /// Stable identity: `key`, or `key:n` (n = position among the records sharing that key) when shared.
    pub id: String,
    /// The 64-bit value at +16 as 16 hex digits (posted as the focus message; four keys are shared by 2 to 5
    /// screens, e.g. TU3 ids 0 / 32 / 49).
    pub key: String,
    /// Byte +4: the world behind this screen blurs while it is on top (the pause crossbar has it set).
    pub blur: bool,
    /// Byte +0: open suspends gameplay (context 3), close resumes it.
    #[serde(default)]
    pub suspends_gameplay: bool,
    /// Byte +1: the chyron shows while it is open.
    #[serde(default)]
    pub chyron: bool,
    /// Word +8: HUD mode while it is the main stack's top (0 on, 1 off, 2 online HUDs only, 3 keep).
    #[serde(default = "hud_off")]
    pub hud: i32,
    /// Byte +12: pad input its own handler leaves goes on to the movie's UpdateInput.
    #[serde(default)]
    pub input_to_movie: bool,
}

fn hud_off() -> i32 {
    1
}

/// One row of the career stats table (the Stats object's +0x18 table, TU3 0x83043AE8, 12-byte records: enabled
/// byte, label string id, value): the loading screen's Stats panel and the Skate Profile read it (doc 31 "Loading
/// screen (FE 52)", research b589).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatRow {
    pub label: String,
    pub enabled: bool,
}

/// One sub-option kind's row: label and helper string ids by column ("" = empty).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubOptionRow {
    pub labels: Vec<String>,
    pub helpers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AptKey {
    pub name: String,
    /// The game input action polled for this key.
    pub action: u32,
    /// The Apt key code in the record (record + 8). Retail does NOT register this: AptUp etc. are set to the
    /// key's INDEX in the table (ScreenManagerReady 825D9450), the same index UpdateInput passes.
    pub apt_key: u32,
    pub released: bool,
    pub kind: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModeTab {
    /// Category name (`SinglePlayer`, `Multiplayer`, ...).
    pub category: String,
    /// Item names in display order.
    pub items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModeMenu {
    /// Stable key (see `mode_keys`): Career, CareerPark, FreePlay, FreePlayPark,
    /// FreeSkateRestricted, or the title without `ID_CROSSBAR_` / `_MODE_TITLE`.
    pub key: String,
    /// The mode's title string id.
    pub title: String,
    /// Row slots per tab (8, or 9 in the skate-park variants).
    pub slots: u32,
    pub tabs: Vec<ModeTab>,
    /// Where the vtable was found (diagnostics only).
    pub vtable: String,
}

/// The Trick Guide screen's tables (FE screen 14, template TrickTutorial; doc 31 "Trick Guide").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrickGuideTables {
    /// The top-level list in menu order: Attrib keys of `scoring_trickcategory` records, 16 hex digits (TU3
    /// 0x830274B0, 8 keys; read by the list fill sub_82619368 and sub_82618C10).
    pub categories: Vec<String>,
    /// Gesture animation names by a slot's gesture id (TU3 0x830273C8, 58 names; GetMenuItemGesturesButtons
    /// 825AE968).
    pub gestures: Vec<String>,
    /// Button names by a slot's button id (TU3 0x83027350, 28 names; sub_82619FC0).
    pub buttons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MenuTables {
    pub schema: u32,
    pub source_sha256: String,
    pub settings_rows: Vec<SettingsRow>,
    /// Row indices per settings screen.
    pub settings_screens: Vec<Vec<u32>>,
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
    pub keys: Vec<AptKey>,
    pub modes: Vec<ModeMenu>,
    /// Screen templates in registration order (empty in tables extracted before they were added).
    #[serde(default)]
    pub templates: Vec<ScreenTemplate>,
    /// FE screens in id order (screen stack info table, 24-byte records; TU3 0x83026088, 72 records, the disc
    /// build has 70). Empty in tables extracted before it was added.
    #[serde(default)]
    pub screens: Vec<FeScreen>,
    /// Game Settings screen titles per settings screen index (string ids; TU3 0x83026E90, 13 pointers, read by
    /// GameSettings_GetScreenTitle 825A6DA8). Empty in tables extracted before they were added.
    #[serde(default)]
    pub settings_titles: Vec<String>,
    /// Crossbar sub-option rows per sub-option kind (0 parks, 1 contacts, 2 host options, 3 Skate Reel, 4 on-screen
    /// help, 5 online team, 6 party): labels (TU3 0x83027958) and helper texts (0x83027A00), 7 x 6 string ids,
    /// "" for an empty slot. Empty in tables extracted before they were added.
    #[serde(default)]
    pub sub_options: Vec<SubOptionRow>,
    /// The Trick Guide's category order and slot name tables. Empty in tables extracted before they were added.
    #[serde(default)]
    pub trick_guide: TrickGuideTables,
    /// The career stats rows in index order (65 in TU3). Empty in tables extracted before they were added.
    #[serde(default)]
    pub career_stats: Vec<StatRow>,
    /// Table addresses in this build (diagnostics only).
    pub addresses: std::collections::BTreeMap<String, String>,
}

impl MenuTables {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("menu tables serialize")
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let tables: Self = serde_json::from_str(text).map_err(|e| format!("menu tables: {e}"))?;
        if tables.schema != SCHEMA {
            return Err(format!("menu tables: schema {} (expected {SCHEMA})", tables.schema));
        }
        Ok(tables)
    }
    pub fn mode(&self, key: &str) -> Option<&ModeMenu> {
        self.modes.iter().find(|m| m.key == key)
    }
}

impl MenuTables {
    /// The engine menu structure with these tables as its defaults: entries keyed
    /// by retail internal names, settings rows by string id, retail links.
    pub fn to_menu_data(&self) -> skate_core::menus::MenuData {
        use skate_core::menus as m;
        let keys: Vec<String> =
            self.settings_rows.iter().enumerate().map(|(i, r)| m::setting_key(i, &r.id)).collect();
        m::MenuData {
            categories: self
                .categories
                .iter()
                .map(|c| m::Category { id: c.name.clone(), label: c.label.clone(), icon: m::Icon::Retail(c.icon.clone()) })
                .collect(),
            items: self
                .items
                .iter()
                .map(|i| m::Item {
                    id: i.name.clone(),
                    label: i.label.clone(),
                    icon: m::Icon::Retail(i.icon.clone()),
                    help: i.help.clone(),
                    sub_option_kind: i.sub_option_kind,
                })
                .collect(),
            settings: self
                .settings_rows
                .iter()
                .zip(&keys)
                .map(|(r, key)| m::Setting {
                    id: key.clone(),
                    label: r.id.clone(),
                    widget: m::Widget::parse(&r.widget).unwrap_or(m::Widget::Option),
                    link: m::retail_link(key),
                })
                .collect(),
            screens: self
                .settings_screens
                .iter()
                .map(|rows| rows.iter().filter_map(|r| keys.get(*r as usize).cloned()).collect())
                .collect(),
            modes: self
                .modes
                .iter()
                .map(|mode| m::Mode {
                    key: mode.key.clone(),
                    title: mode.title.clone(),
                    slots: mode.slots as usize,
                    tabs: mode
                        .tabs
                        .iter()
                        .map(|t| m::Tab { category: t.category.clone(), items: t.items.clone() })
                        .collect(),
                })
                .collect(),
            // The Challenge Map tree comes from the vault collections, not these tables (added by the game's loader).
            map_nodes: Vec::new(), map_pictures: Default::default(), map_markers: Default::default(), map_debug: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Data test: the tables `tools/menu-tables` wrote from the user's disc parse and give a menu registry.
    #[test]
    #[ignore = "requires menu-tables.json from the user's disc (SKATE3_MENU_TABLES=<file>)"]
    fn the_extracted_tables_give_a_menu_registry() {
        let path = std::env::var_os("SKATE3_MENU_TABLES").expect("set SKATE3_MENU_TABLES");
        let tables = MenuTables::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let data = tables.to_menu_data();
        eprintln!(
            "{} items, {} settings rows, {} screens, {} modes, {} FE screens, {} trick guide categories",
            tables.items.len(),
            tables.settings_rows.len(),
            tables.settings_screens.len(),
            tables.modes.len(),
            tables.screens.len(),
            tables.trick_guide.categories.len()
        );
        assert!(!tables.items.is_empty() && !tables.settings_rows.is_empty() && !tables.modes.is_empty());
        let _ = data;
    }
}
