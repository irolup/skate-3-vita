//! The retail menu registry (`skate_core::menus`) with the structure from setup's `private/menu-tables.json` (the
//! user's own executable, `tools/menu-tables`). skate_vita: skate-game `retail_menus.rs` (PR #52 `fad5aae`) without
//! Bevy and without mods: `RetailMenus::new` (the engine handlers), `act` and `read` are upstream's; the PC overlay
//! handlers (`OVERLAY_ITEMS`), the mod commands and events, the stored settings and the engine value bindings are not
//! taken here (the Vita's own glue binds the settings rows).
use skate_core::menus::{EntryRef, Handler, MenuAction, MenuContext, MenuData, MenuRegistry, Outcome, Owner};

pub const TABLES: &str = "private/menu-tables.json";

/// Owner prefix of the engine handlers that push a retail FE screen (by its screen id).
pub const SCREEN: &str = "screen:";

/// FE screen ids (screen stack info table keys; upstream `fe_backdrop.rs`).
pub const PAUSE_SCREEN: &str = "b4b7976b40b3e837";
pub const SETTINGS_SCREEN: &str = "0ca9bfe5c1ba4995";
pub const TRICK_GUIDE_SCREEN: &str = "5d1ecaaf6c7275bf";
pub const VIDEO_CALIBRATION_SCREEN: &str = "1f3e2788f93b6a2e";
pub const CHALLENGE_MAP_SCREEN: &str = "1e146362e71471a9";
/// The Extras screen (TU3 FE 18, template 15 Extras, b320), the text entry (FE 46, no template: retail shows the
/// system keyboard, b305) and the text viewer (FE 3, ToSScreen, template 28, b322).
pub const EXTRAS_SCREEN: &str = "62b6afde0ba6567b";
pub const TEXT_ENTRY_SCREEN: &str = "47ddadb752e051b6";
pub const TEXT_VIEWER_SCREEN: &str = "bd688a2a5602e595";
/// The Skate Profile screen (TU3 FE 61, template 106 OfflineProfile, b308).
pub const OFFLINE_PROFILE_SCREEN: &str = "f17ccf55ee91b4f7";

/// Retail pause items whose retail FE screen is ported: the crossbar pushes it (GameSettings -> FE 45, the Game
/// Settings screen; screen id = its info record key).
pub const SCREEN_ITEMS: [(&str, &str); 1] = [("GameSettings", SETTINGS_SCREEN)];

pub struct RetailMenus {
    pub registry: MenuRegistry,
    /// The current mode and facts (set by the systems that own them).
    pub context: MenuContext,
    /// Whether the retail tables were found (setup ran the extraction).
    pub loaded: bool,
}

impl RetailMenus {
    pub fn new(data: MenuData, loaded: bool) -> Self {
        let mut registry = MenuRegistry::new(data);
        for (item, screen) in SCREEN_ITEMS {
            if registry.data().item(item).is_some() {
                let owner = Owner::Engine(format!("{SCREEN}{screen}"));
                registry.register_handler(EntryRef::item(item), Handler { owner, select: true, highlight: false, callback: None });
            }
        }
        Self { registry, context: MenuContext::new("FreePlay"), loaded }
    }

    /// Apply a menu action in the current context.
    pub fn act(&mut self, action: MenuAction) -> Outcome {
        let context = self.context.clone();
        self.registry.dispatch(action, &context)
    }
}

/// Reads the setup output through `read` (a path relative to the asset root); absent or invalid = an empty
/// structure.
pub fn read_with(read: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> (MenuData, bool) {
    match read(TABLES)
        .and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()))
        .and_then(|t| skate_data::menu_tables::MenuTables::from_json(&t))
    {
        Ok(tables) => (tables.to_menu_data(), true),
        Err(_) => (MenuData::default(), false),
    }
}

/// [`read_with`] from an asset folder (upstream's `read`).
pub fn read(root: &std::path::Path) -> (MenuData, bool) {
    read_with(&|path: &str| std::fs::read(root.join(path)).map_err(|e| e.to_string()))
}
