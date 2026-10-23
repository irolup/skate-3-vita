//! Front-end menu structure and hooks (pause menu "crossbar" and Game Settings).
//!
//! The structure is data: the retail tables extracted at setup (categories,
//! items, settings rows and screens, per-mode tab lists) are the defaults, every
//! entry is keyed by its retail internal name (`GameSettings`, `SkateFeed`,
//! `ID_GAMESETTINGS_SFXVOLUME`, `SinglePlayer`, ...), and mods edit it through
//! owner-tagged layers that are dropped again when the mod is disabled.
//!
//! Hooks per entry: `visible`, `enabled`, `on_select` (an interceptor may ask
//! for a confirmation first), `on_highlight`, and value get/set for settings
//! rows. An entry without a handler is shown greyed (retail's own disabled row
//! state): engine features and mods enable an entry by registering a handler.
//!
//! Engine-independent: no rendering, file I/O or ECS. Actions and events use
//! stable string ids so they can be serialised (multiplayer-ready).
use std::collections::BTreeMap;
use std::sync::Arc;

pub use crate::menu_values::{Direction, Display, ValueRule};

// ---------------------------------------------------------------- ids

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    Category,
    Item,
    Setting,
    /// Challenge Map nodes (FE 17) by vault class; ids are vault keys, greyed until a handler exists (b244, b253).
    MapListing,
    MapFilter,
    MapCategory,
    FeLocation,
    World,
    Challenge,
    MapCommand,
    CodeUnlock,
}

impl EntryKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Item => "item",
            Self::Setting => "setting",
            Self::MapListing => "map_listing",
            Self::MapFilter => "map_filter",
            Self::MapCategory => "map_category",
            Self::FeLocation => "fe_locations",
            Self::World => "world",
            Self::Challenge => "challenges",
            Self::MapCommand => "map_commands",
            Self::CodeUnlock => "code_unlocks",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "category" => Self::Category,
            "item" => Self::Item,
            "setting" => Self::Setting,
            "map_listing" => Self::MapListing,
            "map_filter" => Self::MapFilter,
            "map_category" => Self::MapCategory,
            "fe_locations" => Self::FeLocation,
            "world" => Self::World,
            "challenges" => Self::Challenge,
            "map_commands" => Self::MapCommand,
            "code_unlocks" => Self::CodeUnlock,
            _ => return None,
        })
    }
    /// A Challenge Map node kind.
    pub fn is_map(self) -> bool {
        !matches!(self, Self::Category | Self::Item | Self::Setting)
    }
    /// A Challenge Map folder (tab or folder): enabled when one of its children is.
    pub fn is_map_folder(self) -> bool {
        matches!(self, Self::MapListing | Self::MapFilter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryRef {
    pub kind: EntryKind,
    pub id: String,
}

impl EntryRef {
    pub fn category(id: &str) -> Self {
        Self { kind: EntryKind::Category, id: id.into() }
    }
    pub fn item(id: &str) -> Self {
        Self { kind: EntryKind::Item, id: id.into() }
    }
    pub fn setting(id: &str) -> Self {
        Self { kind: EntryKind::Setting, id: id.into() }
    }
    /// A Challenge Map node, `kind` one of the map kinds, `key` its vault key.
    pub fn map(kind: EntryKind, key: &str) -> Self {
        Self { kind, id: key.into() }
    }
    /// The hook id form `<vault class>:<key>` (`fe_locations:dist_downtown_aletown`).
    pub fn hook_id(&self) -> String {
        format!("{}:{}", self.kind.name(), self.id)
    }
}

/// Who registered a hook or an edit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Owner {
    /// Retail default rules (from the decoded retail code).
    Retail,
    /// An engine feature, by name.
    Engine(String),
    /// A mod, by id.
    Mod(String),
}

// ---------------------------------------------------------------- rules

/// Facts the host publishes (missing = 0). Retail rules read these.
pub mod facts {
    /// Number of SFX packs (retail hides and greys the SFX pack row at <= 1).
    pub const SFX_PACKS: &str = "sfx_packs";
    /// 1 when Project 10 is unlocked (retail skips the item otherwise).
    pub const PROJECT10_UNLOCKED: &str = "project10_unlocked";
    /// Retail GameMode global (1 single player, 4 / 5 online, 0 / 3 / 4 free skate).
    pub const GAME_MODE: &str = "game_mode";
    /// 1 when the player turned on Debug Menus (PC overlay Extras, not retail): retail's debug-only menus show.
    pub const DEBUG_MENUS: &str = "debug_menus";
    /// Completed challenges (retail `ChallengeStatus.GetNumCompletedChallenges`; ours has none yet).
    pub const COMPLETED_CHALLENGES: &str = "completed_challenges";
    /// 1 when the skate park being edited has unsaved changes.
    pub const PARK_DIRTY: &str = "park_dirty";
    /// 1 when the park can be saved (retail: a park-state flag and a mode check).
    pub const PARK_SAVEABLE: &str = "park_saveable";
    /// 1 in a downloaded community park (retail mode state 11 plus checks).
    pub const PARK_COMMUNITY: &str = "park_community";
    /// 1 when Skate Feed settings are available (retail sub_8245A2D0).
    pub const SKATE_FEED_AVAILABLE: &str = "skate_feed_available";
    /// Retail front-end state (2 = the boot front end: settings screen 0, video screen 8).
    pub const FE_STATE: &str = "fe_state";
    /// 1 in an online session state (retail sub_82743FC0).
    pub const ONLINE_SESSION: &str = "online_session";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A data rule (mods can send these as tables; engine code can also register
/// native predicates).
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Always,
    Never,
    Fact { name: String, cmp: Cmp, value: f64 },
    /// True in one of these mode keys.
    Mode(Vec<String>),
    All(Vec<Rule>),
    Any(Vec<Rule>),
    Not(Box<Rule>),
}

impl Rule {
    pub fn fact(name: &str, cmp: Cmp, value: f64) -> Self {
        Self::Fact { name: name.into(), cmp, value }
    }
    pub fn eval(&self, ctx: &MenuContext) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Fact { name, cmp, value } => {
                let v = ctx.fact(name);
                match cmp {
                    Cmp::Eq => v == *value,
                    Cmp::Ne => v != *value,
                    Cmp::Lt => v < *value,
                    Cmp::Le => v <= *value,
                    Cmp::Gt => v > *value,
                    Cmp::Ge => v >= *value,
                }
            }
            Self::Mode(modes) => modes.iter().any(|m| *m == ctx.mode),
            Self::All(rules) => rules.iter().all(|r| r.eval(ctx)),
            Self::Any(rules) => rules.iter().any(|r| r.eval(ctx)),
            Self::Not(rule) => !rule.eval(ctx),
        }
    }
}

pub type Predicate = Arc<dyn Fn(&MenuContext) -> bool + Send + Sync>;

#[derive(Clone)]
pub enum Condition {
    Rule(Rule),
    Native(Predicate),
}

impl Condition {
    pub fn eval(&self, ctx: &MenuContext) -> bool {
        match self {
            Self::Rule(rule) => rule.eval(ctx),
            Self::Native(f) => f(ctx),
        }
    }
}

impl std::fmt::Debug for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rule(rule) => write!(f, "{rule:?}"),
            Self::Native(_) => f.write_str("Native"),
        }
    }
}

/// The current menu context: mode key (`Career`, `FreePlayPark`, ...) and facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuContext {
    pub mode: String,
    pub facts: BTreeMap<String, f64>,
}

impl MenuContext {
    pub fn new(mode: &str) -> Self {
        Self { mode: mode.into(), facts: BTreeMap::new() }
    }
    pub fn with(mut self, name: &str, value: f64) -> Self {
        self.facts.insert(name.into(), value);
        self
    }
    pub fn fact(&self, name: &str) -> f64 {
        self.facts.get(name).copied().unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------- structure

#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    /// A frame label of the retail icon sprite (`map`, `settings`, `skatefeed`, ...).
    Retail(String),
    /// A mod-supplied image, as a path inside the mod (drawn by a later milestone).
    Image(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Widget {
    /// A link to another screen or an action.
    Option,
    Slider,
    Selector,
}

impl Widget {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "option" => Some(Self::Option),
            "slider" => Some(Self::Slider),
            "selector" => Some(Self::Selector),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub id: String,
    pub label: String,
    pub icon: Icon,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub icon: Icon,
    pub help: String,
    /// Retail sub-option list kind (-1 = none).
    pub sub_option_kind: i32,
}

/// Which screen an `option` row opens.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Screen(usize),
    /// The first matching rule wins (retail's Video screen choice); the last
    /// entry should use `Rule::Always`.
    Choose(Vec<(Rule, usize)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub id: String,
    pub label: String,
    pub widget: Widget,
    pub link: Option<Link>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub category: String,
    pub items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    pub key: String,
    pub title: String,
    /// Retail row slots per tab (8; 9 in the park variants).
    pub slots: usize,
    pub tabs: Vec<Tab>,
}

/// The menu structure (retail tables or the result of applying edits).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuData {
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
    pub settings: Vec<Setting>,
    /// Setting ids per screen (retail screen index = position).
    pub screens: Vec<Vec<String>>,
    pub modes: Vec<Mode>,
    /// The Challenge Map tree (vault `map_listing` parent chain and its leaves), in display order per parent.
    pub map_nodes: Vec<MapNode>,
    /// The Challenge Map picture per location key (the `world` record's tMapInfo name, `map_skateschool`); a location
    /// not listed shows the default map. Data: mods may change it.
    pub map_pictures: std::collections::BTreeMap<String, String>,
    /// The Challenge Map marker per location key (icon, world point, its world's map bounds); data, mods may change
    /// it. A location without one draws no marker.
    pub map_markers: std::collections::BTreeMap<String, MapMarker>,
    /// Challenge Map listings flagged `IsDebug` in the vault (research b479): hidden unless Debug Menus is on.
    pub map_debug: std::collections::BTreeSet<String>,
}

/// One Challenge Map location marker (research b263 / b264 / b265): the icon texture name (`mini_teleport`), the
/// world point (the locator's x / z plus its world record's offset) and that world's tMapInfo bounds b1..b6.
#[derive(Clone, Debug, PartialEq)]
pub struct MapMarker {
    pub icon: String,
    pub point: [f32; 2],
    pub bounds: [i32; 6],
}

impl MapMarker {
    /// Map pixels relative to the map centre (`sub_8268DB68`): size b5 x b6 over the bounds b2..b1 (x) and b4..b3
    /// (z), shrunk to 99% / 95%.
    pub fn projected(&self) -> [f32; 2] {
        let b = self.bounds.map(|v| v as f32);
        let mid_x = (b[0] - b[1]) * 0.5 + b[1];
        let mid_y = (b[2] - b[3]) * 0.5 + b[3];
        let x = if mid_x != b[1] { (b[4] * 0.5 / (mid_x - b[1])) * (self.point[0] - mid_x) * 0.99 } else { 0.0 };
        let y = if mid_y != b[3] { (b[5] * 0.5 / (mid_y - b[3])) * (self.point[1] - mid_y) * 0.95 } else { 0.0 };
        [x, y]
    }
}

/// One Challenge Map node: its entry (kind + vault key), its parent folder, its label string id and sort order.
#[derive(Clone, Debug, PartialEq)]
pub struct MapNode {
    pub entry: EntryRef,
    pub parent: Option<EntryRef>,
    pub label: String,
    pub order: i32,
}

impl MenuData {
    pub fn mode(&self, key: &str) -> Option<&Mode> {
        self.modes.iter().find(|m| m.key == key)
    }
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
    pub fn category(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|i| i.id == id)
    }
    pub fn setting(&self, id: &str) -> Option<&Setting> {
        self.settings.iter().find(|i| i.id == id)
    }
    pub fn contains(&self, entry: &EntryRef) -> bool {
        match entry.kind {
            EntryKind::Category => self.category(&entry.id).is_some(),
            EntryKind::Item => self.item(&entry.id).is_some(),
            EntryKind::Setting => self.setting(&entry.id).is_some(),
            _ => self.map_node(entry).is_some(),
        }
    }
    pub fn map_node(&self, entry: &EntryRef) -> Option<&MapNode> {
        self.map_nodes.iter().find(|n| n.entry == *entry)
    }
    /// A Challenge Map folder's children in display order (sort order, then data order).
    pub fn map_children(&self, parent: &EntryRef) -> Vec<&MapNode> {
        let mut children: Vec<&MapNode> = self.map_nodes.iter().filter(|n| n.parent.as_ref() == Some(parent)).collect();
        children.sort_by_key(|n| n.order);
        children
    }
}

/// Stable key of a settings row: its string id, or `GAMESETTINGS_ROW_<n>` for
/// retail's blank rows.
pub fn setting_key(index: usize, string_id: &str) -> String {
    if string_id.starts_with("ID_") {
        string_id.to_owned()
    } else {
        format!("GAMESETTINGS_ROW_{index}")
    }
}

// ---------------------------------------------------------------- edits

/// Where an item or category goes. `modes` empty = every mode that has the
/// tab (for items) or every mode (for categories).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    pub modes: Vec<String>,
    /// Item: the tab (category id) it goes into.
    pub category: Option<String>,
    /// 0-based position; `None` = at the end (new) or unchanged (existing).
    pub position: Option<usize>,
}

/// One structural change by an owner. Add-or-override: fields left `None`
/// keep the current value.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Category { id: String, label: Option<String>, icon: Option<Icon>, place: Option<Placement> },
    Item {
        id: String,
        label: Option<String>,
        icon: Option<Icon>,
        help: Option<String>,
        place: Option<Placement>,
    },
    Setting {
        id: String,
        label: Option<String>,
        widget: Option<Widget>,
        link: Option<Link>,
        /// Screen index and position (`None` = end).
        place: Option<(usize, Option<usize>)>,
    },
    /// Remove an entry from the lists (in `modes`, or everywhere when empty).
    Hide { entry: EntryRef, modes: Vec<String> },
}

fn place_in(list: &mut Vec<String>, id: &str, position: Option<usize>) {
    let existing = list.iter().position(|x| x == id);
    match (existing, position) {
        (Some(_), None) => {}
        (Some(i), Some(p)) => {
            let v = list.remove(i);
            list.insert(p.min(list.len()), v);
        }
        (None, p) => {
            let at = p.unwrap_or(list.len()).min(list.len());
            list.insert(at, id.to_owned());
        }
    }
}

impl MenuData {
    fn apply(&mut self, edit: &Edit) -> Result<(), String> {
        match edit {
            Edit::Category { id, label, icon, place } => {
                if self.category(id).is_none() {
                    self.categories.push(Category {
                        id: id.clone(),
                        label: label.clone().unwrap_or_else(|| id.clone()),
                        icon: icon.clone().unwrap_or(Icon::Retail("options".into())),
                    });
                }
                let category = self.categories.iter_mut().find(|c| c.id == *id).unwrap();
                if let Some(label) = label {
                    category.label = label.clone();
                }
                if let Some(icon) = icon {
                    category.icon = icon.clone();
                }
                if let Some(place) = place {
                    for mode in self.modes.iter_mut().filter(|m| place.modes.is_empty() || place.modes.contains(&m.key)) {
                        let mut order: Vec<String> = mode.tabs.iter().map(|t| t.category.clone()).collect();
                        place_in(&mut order, id, place.position);
                        let mut tabs = std::mem::take(&mut mode.tabs);
                        mode.tabs = order
                            .iter()
                            .map(|c| match tabs.iter().position(|t| t.category == *c) {
                                Some(i) => tabs.remove(i),
                                None => Tab { category: c.clone(), items: Vec::new() },
                            })
                            .collect();
                    }
                }
            }
            Edit::Item { id, label, icon, help, place } => {
                if self.item(id).is_none() {
                    if place.as_ref().and_then(|p| p.category.as_ref()).is_none() {
                        return Err(format!("new item {id} needs a category"));
                    }
                    self.items.push(Item {
                        id: id.clone(),
                        label: label.clone().unwrap_or_else(|| id.clone()),
                        icon: icon.clone().unwrap_or(Icon::Retail("extras".into())),
                        help: help.clone().unwrap_or_default(),
                        sub_option_kind: -1,
                    });
                }
                let item = self.items.iter_mut().find(|c| c.id == *id).unwrap();
                if let Some(label) = label {
                    item.label = label.clone();
                }
                if let Some(icon) = icon {
                    item.icon = icon.clone();
                }
                if let Some(help) = help {
                    item.help = help.clone();
                }
                if let Some(place) = place {
                    for mode in self.modes.iter_mut().filter(|m| place.modes.is_empty() || place.modes.contains(&m.key)) {
                        let current = mode.tabs.iter().position(|t| t.items.contains(id));
                        let target = match &place.category {
                            Some(c) => mode.tabs.iter().position(|t| t.category == *c),
                            None => current,
                        };
                        let Some(target) = target else { continue };
                        if let Some(current) = current.filter(|c| *c != target) {
                            mode.tabs[current].items.retain(|x| x != id);
                        }
                        place_in(&mut mode.tabs[target].items, id, place.position);
                    }
                }
            }
            Edit::Setting { id, label, widget, link, place } => {
                if self.setting(id).is_none() {
                    let Some(widget) = widget else { return Err(format!("new setting {id} needs a widget")) };
                    if place.is_none() {
                        return Err(format!("new setting {id} needs a screen"));
                    }
                    self.settings.push(Setting { id: id.clone(), label: label.clone().unwrap_or_else(|| id.clone()), widget: *widget, link: None });
                }
                let setting = self.settings.iter_mut().find(|c| c.id == *id).unwrap();
                if let Some(label) = label {
                    setting.label = label.clone();
                }
                if let Some(widget) = widget {
                    setting.widget = *widget;
                }
                if let Some(link) = link {
                    setting.link = Some(link.clone());
                }
                if let Some((screen, position)) = place {
                    let rows = self.screens.get_mut(*screen).ok_or_else(|| format!("setting {id}: no screen {screen}"))?;
                    place_in(rows, id, *position);
                }
            }
            Edit::Hide { entry, modes } => {
                let in_mode = |m: &Mode| modes.is_empty() || modes.contains(&m.key);
                match entry.kind {
                    EntryKind::Category => {
                        for mode in self.modes.iter_mut().filter(|m| in_mode(m)) {
                            mode.tabs.retain(|t| t.category != entry.id);
                        }
                    }
                    EntryKind::Item => {
                        for mode in self.modes.iter_mut().filter(|m| in_mode(m)) {
                            for tab in &mut mode.tabs {
                                tab.items.retain(|x| *x != entry.id);
                            }
                        }
                    }
                    EntryKind::Setting => {
                        for rows in &mut self.screens {
                            rows.retain(|x| *x != entry.id);
                        }
                    }
                    // A Challenge Map node goes with everything under it (the tree roots are the modes).
                    _ => {
                        let mut gone = vec![entry.clone()];
                        let mut i = 0;
                        while i < gone.len() {
                            let parent = gone[i].clone();
                            gone.extend(self.map_nodes.iter().filter(|n| n.parent.as_ref() == Some(&parent)).map(|n| n.entry.clone()));
                            i += 1;
                        }
                        self.map_nodes.retain(|n| !gone.contains(&n.entry));
                    }
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- hooks

#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Float(f64),
}

pub type Callback = Arc<dyn Fn(&MenuContext, &MenuEvent) + Send + Sync>;
pub type Getter = Arc<dyn Fn() -> SettingValue + Send + Sync>;
pub type Setter = Arc<dyn Fn(&SettingValue) + Send + Sync>;

/// A select / highlight handler. `callback: None` = forward the event to the
/// owner (how mods receive theirs).
#[derive(Clone)]
pub struct Handler {
    pub owner: Owner,
    pub select: bool,
    pub highlight: bool,
    pub callback: Option<Callback>,
}

#[derive(Clone)]
pub enum ValueHook {
    Native { get: Getter, set: Setter },
    /// A value the registry keeps (mod-bound rows).
    Stored(SettingValue),
}

/// A confirmation popup (language string ids).
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub title: String,
    pub description: String,
    pub yes: String,
    pub no: String,
}

/// Runs before the handler: when `when` holds, the select first asks `popup`.
#[derive(Clone, Debug, PartialEq)]
pub struct Interceptor {
    pub owner: Owner,
    pub when: Rule,
    pub popup: Popup,
}

#[derive(Clone, Default)]
struct Hooks {
    handlers: Vec<Handler>,
    visible: Vec<(Owner, Condition)>,
    enabled: Vec<(Owner, Condition)>,
    interceptors: Vec<Interceptor>,
    values: Vec<(Owner, ValueHook)>,
    /// Left / Right rules (retail defaults as `Owner::Retail`, see `menu_values`).
    rules: Vec<(Owner, ValueRule)>,
}

// ---------------------------------------------------------------- events

/// A menu action with stable ids (serialisable; the host applies it).
#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    Select { entry: EntryRef },
    /// The answer to a confirmation popup for `entry`.
    Confirm { entry: EntryRef, accepted: bool },
    Highlight { entry: EntryRef },
    SetValue { entry: EntryRef, value: SettingValue },
    /// A Left / Right press on a settings row: the row's rule turns it into a `SetValue`.
    Step { entry: EntryRef, direction: Direction },
}

impl MenuAction {
    pub fn entry(&self) -> &EntryRef {
        match self {
            Self::Select { entry } | Self::Confirm { entry, .. } | Self::Highlight { entry } | Self::SetValue { entry, .. } | Self::Step { entry, .. } => entry,
        }
    }
}

/// What a handler receives (the action plus the mode it happened in).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEvent {
    pub mode: String,
    pub action: MenuAction,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Not in the current menu, hidden or greyed: nothing happens.
    Ignored,
    /// Show this popup; answer with `MenuAction::Confirm`.
    Confirm(Popup),
    /// A native handler ran.
    Handled(Owner),
    /// The host forwards the event to this owner (a mod).
    Forward(Owner, MenuEvent),
    /// The popup was declined.
    Declined,
    /// A settings link: open this screen.
    OpenScreen(usize),
}

/// One visible row of a tab or screen, as the menu shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub enabled: bool,
}

// ---------------------------------------------------------------- registry

/// Settings links followed when deciding whether a link row is enabled.
const MAX_LINK_DEPTH: usize = 4;
/// Deepest Challenge Map folder nesting the folder rule follows (the retail tree is 3-4 deep).
const MAX_MAP_DEPTH: usize = 8;

pub struct MenuRegistry {
    base: MenuData,
    data: MenuData,
    edits: Vec<(Owner, Edit)>,
    hooks: BTreeMap<EntryRef, Hooks>,
}

impl MenuRegistry {
    /// A registry over `base` (the retail tables) with retail's default rules.
    pub fn new(base: MenuData) -> Self {
        let mut registry = Self { data: base.clone(), base, edits: Vec::new(), hooks: BTreeMap::new() };
        registry.install_retail_rules();
        registry
    }

    pub fn data(&self) -> &MenuData {
        &self.data
    }

    fn hooks_mut(&mut self, entry: EntryRef) -> &mut Hooks {
        self.hooks.entry(entry).or_default()
    }

    /// Retail rules from the decoded code (docs/hails-additions retail menus doc),
    /// registered as `Owner::Retail`; any owner can stack its own on top.
    fn install_retail_rules(&mut self) {
        use facts::*;
        let retail = Owner::Retail;
        let not_online = Rule::Not(Box::new(Rule::Any(vec![
            Rule::fact(GAME_MODE, Cmp::Eq, 4.0),
            Rule::fact(GAME_MODE, Cmp::Eq, 5.0),
        ])));
        let packs = Rule::fact(SFX_PACKS, Cmp::Gt, 1.0);
        let parks = vec!["CareerPark".to_owned(), "FreePlayPark".to_owned()];
        let in_park = Rule::Mode(parks.clone());
        let not_in_park = Rule::Not(Box::new(in_park.clone()));
        let rules: Vec<(EntryRef, Option<Rule>, Option<Rule>)> = vec![
            // Settings: GameSettings_GetNumOptions stops at the SFX pack row with <= 1 pack;
            // IsItemEnabled (sub_8260D648) greys it, Difficulty when online, Skate Feed when unavailable.
            (EntryRef::setting("ID_GAMESETTINGS_SFXPACK"), Some(packs.clone()), Some(packs)),
            (EntryRef::setting("ID_GAMESETTINGS_DIFFICULTY_SETTINGS"), None, Some(not_online)),
            (EntryRef::setting("ID_GAMESETTINGS_SKATEFEED_SETTINGS"), None, Some(Rule::fact(SKATE_FEED_AVAILABLE, Cmp::Ne, 0.0))),
            // Crossbar: the row getter skips Project10 until unlocked.
            (EntryRef::item("Project10"), Some(Rule::fact(PROJECT10_UNLOCKED, Cmp::Ne, 0.0)), None),
            // Park IsItemEnabled (sub_8261FB18): SkateReel / SkateWith off in a park; SavePark,
            // RatePark and FlagSkatePark by park state.
            (EntryRef::item("SkateReel"), None, Some(not_in_park.clone())),
            (EntryRef::item("SkateWith"), None, Some(not_in_park)),
            (EntryRef::item("SavePark"), None, Some(Rule::fact(PARK_SAVEABLE, Cmp::Ne, 0.0))),
            (EntryRef::item("RatePark"), None, Some(Rule::fact(PARK_COMMUNITY, Cmp::Ne, 0.0))),
            (EntryRef::item("FlagSkatePark"), None, Some(Rule::fact(PARK_COMMUNITY, Cmp::Ne, 0.0))),
            // Challenge Map tabs: each map_listing's tab Lua (Hash_3E294F92C056A144, skatercollections): Challenges
            // and All are hidden in free skate, Completed also while nothing is completed; Locations always shows.
            (EntryRef::map(EntryKind::MapListing, "progression_active"), Some(not_free_skate()), None),
            (EntryRef::map(EntryKind::MapListing, "progression_completed"),
                Some(Rule::All(vec![not_free_skate(), Rule::fact(COMPLETED_CHALLENGES, Cmp::Gt, 0.0)])), None),
        ];
        // Listings flagged IsDebug (82D0C0E0 skips them; b479) show only with Debug Menus (ours, PC overlay Extras).
        let debug: Vec<EntryRef> = self.data.map_debug.iter().map(|id| EntryRef::map(EntryKind::MapListing, id)).collect();
        for entry in debug {
            self.hooks_mut(entry).visible.push((retail.clone(), Condition::Rule(Rule::fact(DEBUG_MENUS, Cmp::Ne, 0.0))));
        }
        for (entry, visible, enabled) in rules {
            let hooks = self.hooks_mut(entry);
            if let Some(rule) = visible {
                hooks.visible.push((retail.clone(), Condition::Rule(rule)));
            }
            if let Some(rule) = enabled {
                hooks.enabled.push((retail.clone(), Condition::Rule(rule)));
            }
        }
        // Settings value rules (sub_8260C258), keyed by string id so the disc and TU3 tables agree.
        let ids: Vec<String> = self.base.settings.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            if let Some(rule) = crate::menu_values::retail_rule(&id) {
                self.hooks_mut(EntryRef::setting(&id)).rules.push((retail.clone(), rule));
            }
        }
        // Park select hook (sub_8261F9D8): leaving a dirty park through the map asks first.
        for item in ["ChallengeMap", "OnlineChallengeMap"] {
            self.hooks_mut(EntryRef::item(item)).interceptors.push(Interceptor {
                owner: retail.clone(),
                when: Rule::All(vec![in_park.clone(), Rule::fact(PARK_DIRTY, Cmp::Ne, 0.0)]),
                popup: Popup {
                    title: "ID_SKATEPARK_DIRTY_TITLE".into(),
                    description: "ID_SKATEPARK_DIRTY_DESC".into(),
                    yes: "ID_COMMON_YES".into(),
                    no: "ID_COMMON_NO".into(),
                },
            });
        }
    }

    // ------------------------------------------------ registration

    /// Register a select / highlight handler; this is what enables an entry.
    pub fn register_handler(&mut self, entry: EntryRef, handler: Handler) {
        let hooks = self.hooks_mut(entry);
        hooks.handlers.retain(|h| h.owner != handler.owner);
        hooks.handlers.push(handler);
    }
    pub fn set_visible(&mut self, entry: EntryRef, owner: Owner, condition: Condition) {
        let hooks = self.hooks_mut(entry);
        hooks.visible.retain(|(o, _)| *o != owner);
        hooks.visible.push((owner, condition));
    }
    pub fn set_enabled(&mut self, entry: EntryRef, owner: Owner, condition: Condition) {
        let hooks = self.hooks_mut(entry);
        hooks.enabled.retain(|(o, _)| *o != owner);
        hooks.enabled.push((owner, condition));
    }
    pub fn add_interceptor(&mut self, entry: EntryRef, interceptor: Interceptor) {
        let hooks = self.hooks_mut(entry);
        hooks.interceptors.retain(|i| i.owner != interceptor.owner);
        hooks.interceptors.push(interceptor);
    }
    /// Bind a settings row's value (enables a slider / selector row).
    pub fn bind_value(&mut self, entry: EntryRef, owner: Owner, hook: ValueHook) {
        let hooks = self.hooks_mut(entry);
        hooks.values.retain(|(o, _)| *o != owner);
        hooks.values.push((owner, hook));
    }

    /// Set `owner`'s Left / Right rule for a settings row (stacks over retail's).
    pub fn set_rule(&mut self, entry: EntryRef, owner: Owner, rule: ValueRule) {
        let hooks = self.hooks_mut(entry);
        hooks.rules.retain(|(o, _)| *o != owner);
        hooks.rules.push((owner, rule));
    }

    /// Apply a structural edit for `owner` (kept as a layer over the retail data).
    pub fn edit(&mut self, owner: Owner, edit: Edit) -> Result<(), String> {
        let mut data = self.data.clone();
        data.apply(&edit)?;
        self.data = data;
        self.edits.push((owner, edit));
        Ok(())
    }

    /// Drop `owner`'s hooks on one entry (its structural edits stay).
    pub fn remove_owner_from(&mut self, entry: &EntryRef, owner: &Owner) {
        if let Some(hooks) = self.hooks.get_mut(entry) {
            hooks.handlers.retain(|h| h.owner != *owner);
            hooks.visible.retain(|(o, _)| o != owner);
            hooks.enabled.retain(|(o, _)| o != owner);
            hooks.interceptors.retain(|i| i.owner != *owner);
            hooks.values.retain(|(o, _)| o != owner);
            hooks.rules.retain(|(o, _)| o != owner);
        }
    }

    /// Drop everything `owner` registered or edited (mod disabled / feature off).
    pub fn remove_owner(&mut self, owner: &Owner) {
        for hooks in self.hooks.values_mut() {
            hooks.handlers.retain(|h| h.owner != *owner);
            hooks.visible.retain(|(o, _)| o != owner);
            hooks.enabled.retain(|(o, _)| o != owner);
            hooks.interceptors.retain(|i| i.owner != *owner);
            hooks.values.retain(|(o, _)| o != owner);
            hooks.rules.retain(|(o, _)| o != owner);
        }
        self.edits.retain(|(o, _)| o != owner);
        self.data = self.base.clone();
        for (_, edit) in &self.edits {
            // Edits that applied before still apply over the same or a smaller base;
            // one that no longer does is dropped from the view.
            let _ = self.data.apply(edit);
        }
    }

    // ------------------------------------------------ queries

    fn top<'a, T>(list: &'a [(Owner, T)]) -> Option<&'a T> {
        list.last().map(|(_, t)| t)
    }

    pub fn visible(&self, entry: &EntryRef, ctx: &MenuContext) -> bool {
        self.data.contains(entry)
            && self.hooks.get(entry).and_then(|h| Self::top(&h.visible)).is_none_or(|c| c.eval(ctx))
    }

    pub fn has_handler(&self, entry: &EntryRef) -> bool {
        self.hooks.get(entry).is_some_and(|h| h.handlers.iter().any(|h| h.select) || !h.values.is_empty())
    }

    /// Greyed unless a handler (or value binding) exists and the top enabled rule holds.
    /// A settings link is enabled when its screen has an enabled row.
    pub fn enabled(&self, entry: &EntryRef, ctx: &MenuContext) -> bool {
        self.enabled_at(entry, ctx, 0)
    }

    fn enabled_at(&self, entry: &EntryRef, ctx: &MenuContext, depth: usize) -> bool {
        if !self.visible(entry, ctx) {
            return false;
        }
        if let Some(rule) = self.hooks.get(entry).and_then(|h| Self::top(&h.enabled)) {
            if !rule.eval(ctx) {
                return false;
            }
        }
        if self.has_handler(entry) {
            return true;
        }
        if entry.kind == EntryKind::Setting && depth < MAX_LINK_DEPTH {
            if let Some(screen) = self.link_target(&entry.id, ctx) {
                return self.screen_rows_at(screen, ctx, depth + 1).iter().any(|r| r.enabled);
            }
        }
        // A Challenge Map tab / folder opens when something inside it can be used (ours, like the settings link).
        if entry.kind.is_map_folder() && depth < MAX_MAP_DEPTH {
            return self.data.map_children(entry).iter().any(|c| self.enabled_at(&c.entry, ctx, depth + 1));
        }
        false
    }

    fn link_target(&self, setting: &str, ctx: &MenuContext) -> Option<usize> {
        match self.data.setting(setting)?.link.as_ref()? {
            Link::Screen(s) => Some(*s),
            Link::Choose(choices) => choices.iter().find(|(r, _)| r.eval(ctx)).map(|(_, s)| *s),
        }
    }

    /// The tabs (category ids) of the current mode, in order.
    pub fn tabs(&self, ctx: &MenuContext) -> Vec<String> {
        let Some(mode) = self.data.mode(&ctx.mode) else { return Vec::new() };
        mode.tabs
            .iter()
            .filter(|t| self.visible(&EntryRef::category(&t.category), ctx))
            .map(|t| t.category.clone())
            .collect()
    }

    /// The visible rows of a tab in the current mode.
    pub fn tab_rows(&self, category: &str, ctx: &MenuContext) -> Vec<Row> {
        let Some(tab) = self.data.mode(&ctx.mode).and_then(|m| m.tabs.iter().find(|t| t.category == category)) else {
            return Vec::new();
        };
        tab.items
            .iter()
            .map(|id| EntryRef::item(id))
            .filter(|e| self.visible(e, ctx))
            .map(|e| Row { enabled: self.enabled(&e, ctx), id: e.id })
            .collect()
    }

    /// The visible rows of a settings screen.
    pub fn screen_rows(&self, screen: usize, ctx: &MenuContext) -> Vec<Row> {
        self.screen_rows_at(screen, ctx, 0)
    }

    fn screen_rows_at(&self, screen: usize, ctx: &MenuContext, depth: usize) -> Vec<Row> {
        let Some(rows) = self.data.screens.get(screen) else { return Vec::new() };
        rows.iter()
            .map(|id| EntryRef::setting(id))
            .filter(|e| self.visible(e, ctx))
            .map(|e| Row { enabled: self.enabled_at(&e, ctx, depth), id: e.id })
            .collect()
    }

    /// The value in force: the top value layer, else retail's default for the row (shown, not a binding:
    /// it neither enables the row nor takes Left / Right).
    pub fn value(&self, entry: &EntryRef) -> Option<SettingValue> {
        match self.hooks.get(entry).and_then(|h| Self::top(&h.values)) {
            Some(ValueHook::Native { get, .. }) => Some(get()),
            Some(ValueHook::Stored(v)) => Some(v.clone()),
            None if entry.kind == EntryKind::Setting => crate::menu_values::retail_default(&entry.id),
            None => None,
        }
    }

    /// Who owns the value in force (the top value layer).
    pub fn value_owner(&self, entry: &EntryRef) -> Option<&Owner> {
        self.hooks.get(entry)?.values.last().map(|(o, _)| o)
    }

    /// The rule in force for a row (the top owner's).
    pub fn rule(&self, entry: &EntryRef) -> Option<&ValueRule> {
        Self::top(&self.hooks.get(entry)?.rules)
    }

    /// What a value row shows (slider bars or a selector label id).
    pub fn display(&self, entry: &EntryRef) -> Option<Display> {
        Some(self.rule(entry)?.display(&self.value(entry)?))
    }

    /// `owner`'s own binding value (an engine binding below a mod override keeps its value).
    pub fn owner_value(&self, entry: &EntryRef, owner: &Owner) -> Option<SettingValue> {
        match &self.hooks.get(entry)?.values.iter().find(|(o, _)| o == owner)?.1 {
            ValueHook::Native { get, .. } => Some(get()),
            ValueHook::Stored(v) => Some(v.clone()),
        }
    }

    /// Update `owner`'s stored binding without an event (the owner's source changed).
    pub fn set_owner_value(&mut self, entry: &EntryRef, owner: &Owner, value: SettingValue) {
        if let Some((_, ValueHook::Stored(v))) =
            self.hooks.get_mut(entry).and_then(|h| h.values.iter_mut().find(|(o, _)| o == owner))
        {
            *v = value;
        }
    }

    // ------------------------------------------------ dispatch

    /// Apply a menu action in `ctx` and say what the host does next.
    pub fn dispatch(&mut self, action: MenuAction, ctx: &MenuContext) -> Outcome {
        let entry = action.entry().clone();
        let event = MenuEvent { mode: ctx.mode.clone(), action: action.clone() };
        match &action {
            MenuAction::Highlight { .. } => {
                if !self.visible(&entry, ctx) {
                    return Outcome::Ignored;
                }
                self.run(&entry, ctx, &event, |h| h.highlight)
            }
            MenuAction::Select { .. } => {
                if !self.enabled(&entry, ctx) {
                    return Outcome::Ignored;
                }
                let hooks = self.hooks.get(&entry);
                if let Some(i) = hooks.and_then(|h| h.interceptors.iter().rev().find(|i| i.when.eval(ctx))) {
                    return Outcome::Confirm(i.popup.clone());
                }
                if !self.hooks.get(&entry).is_some_and(|h| h.handlers.iter().any(|h| h.select)) {
                    if let Some(screen) = self.link_target(&entry.id, ctx).filter(|_| entry.kind == EntryKind::Setting) {
                        return Outcome::OpenScreen(screen);
                    }
                }
                self.run(&entry, ctx, &event, |h| h.select)
            }
            MenuAction::Confirm { accepted, .. } => {
                if !*accepted {
                    return Outcome::Declined;
                }
                if !self.enabled(&entry, ctx) {
                    return Outcome::Ignored;
                }
                let select = MenuEvent { mode: ctx.mode.clone(), action: MenuAction::Select { entry: entry.clone() } };
                self.run(&entry, ctx, &select, |h| h.select)
            }
            MenuAction::Step { direction, .. } => {
                let (Some(rule), Some(current)) = (self.rule(&entry), self.value(&entry)) else {
                    return Outcome::Ignored;
                };
                let value = rule.step(&current, *direction);
                self.dispatch(MenuAction::SetValue { entry, value }, ctx)
            }
            MenuAction::SetValue { value, .. } => {
                if !self.enabled(&entry, ctx) {
                    return Outcome::Ignored;
                }
                let Some(hooks) = self.hooks.get_mut(&entry) else { return Outcome::Ignored };
                let Some((owner, hook)) = hooks.values.last_mut() else { return Outcome::Ignored };
                match hook {
                    ValueHook::Native { set, .. } => {
                        set(value);
                        Outcome::Handled(owner.clone())
                    }
                    ValueHook::Stored(stored) => {
                        *stored = value.clone();
                        Outcome::Forward(owner.clone(), event)
                    }
                }
            }
        }
    }

    fn run(&self, entry: &EntryRef, ctx: &MenuContext, event: &MenuEvent, wants: impl Fn(&Handler) -> bool) -> Outcome {
        let Some(handler) = self.hooks.get(entry).and_then(|h| h.handlers.iter().rev().find(|h| wants(h))) else {
            return Outcome::Ignored;
        };
        match &handler.callback {
            Some(callback) => {
                callback(ctx, event);
                Outcome::Handled(handler.owner.clone())
            }
            None => Outcome::Forward(handler.owner.clone(), event.clone()),
        }
    }
}

/// Retail settings links and Video screen choice (sub_8260D290): Difficulty ->
/// screen 2, Audio -> 3, Video -> 8 / 7 / 6 / 5 / 4 by front-end state, mode and
/// session, Controls -> 9, Online -> 10, Skate Feed -> 11.
/// Retail's free-skate game modes (0 / 3 / 4).
fn free_skate_rule() -> Rule {
    use facts::GAME_MODE;
    Rule::Any(vec![
        Rule::fact(GAME_MODE, Cmp::Eq, 0.0),
        Rule::fact(GAME_MODE, Cmp::Eq, 3.0),
        Rule::fact(GAME_MODE, Cmp::Eq, 4.0),
    ])
}

fn not_free_skate() -> Rule {
    Rule::Not(Box::new(free_skate_rule()))
}

pub fn retail_link(setting: &str) -> Option<Link> {
    use facts::*;
    let free_skate = free_skate_rule();
    let session = Rule::fact(ONLINE_SESSION, Cmp::Ne, 0.0);
    Some(match setting {
        "ID_GAMESETTINGS_DIFFICULTY_SETTINGS" => Link::Screen(2),
        "ID_GAMESETTINGS_AUDIO_SETTINGS" => Link::Screen(3),
        "ID_GAMESETTINGS_VIDEO_SETTINGS" => Link::Choose(vec![
            (Rule::fact(FE_STATE, Cmp::Eq, 2.0), 8),
            (Rule::All(vec![free_skate.clone(), session.clone()]), 7),
            (free_skate, 6),
            (session, 5),
            (Rule::Always, 4),
        ]),
        "ID_GAMESETTINGS_CONTROL_SETTINGS" => Link::Screen(9),
        "ID_GAMESETTINGS_ONLINE_SETTINGS" => Link::Screen(10),
        "ID_GAMESETTINGS_SKATEFEED_SETTINGS" => Link::Screen(11),
        _ => return None,
    })
}

#[cfg(test)]
#[path = "menus_tests.rs"]
mod tests;
