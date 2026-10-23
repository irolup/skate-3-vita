//! The retail Challenge Map screen's navigation (FE screen 17, research b255 / b256), over the menu registry's map
//! nodes: the tab (`mgr+0x198`, clamped, no wrap), the folder stack (`mgr+0xE5B4`), the highlighted row (`+0xD8`) and
//! the selected one (`mgr+0xE5E8`). Engine-independent: the host (skate-game `challenge_map_host`) turns the movie's
//! natives and picker callbacks into these calls and posts the returned screen states (`SetState(n)`).
use crate::menus::{EntryKind, EntryRef, MenuContext, MenuRegistry};

/// The movie's `SetState` codes the C++ posts (b256 "Movie refresh") [code + data].
pub mod state {
    pub const RELOAD_LIST: i32 = 10;
    pub const LIST_PANEL: i32 = 21;
    pub const DEFAULT_MAP: i32 = 33;
    pub const MAP_TEXTURE: i32 = 34;
    pub const LIST_TITLE_OFF: i32 = 41;
    pub const LIST_TITLE: i32 = 80;
    pub const DISTRICT_TITLE: i32 = 90;
    pub const SELECT_TAB: i32 = 100;
    pub const CURSOR: i32 = 110;
    pub const LEGEND_ON: i32 = 120;
    pub const LEGEND_OFF: i32 = 130;
}

/// One row of the current list (GetStuffMenuItems' per-row values).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub entry: EntryRef,
    /// The label string id (the host localizes it).
    pub label: String,
    /// Greyed unless something handles it (ours: the registry's rule; retail's `!vt+0x28` is "not a header").
    pub enabled: bool,
}

/// What a select or back press asks the host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Nothing (a greyed row, an empty folder, a refresh pending).
    None,
    /// The list changed (folder pushed / popped, tab switched): post these states.
    Refresh(Vec<i32>),
    /// A leaf was selected: dispatch it through the registry (its handler decides: a teleport, a mod).
    Select(EntryRef),
    /// B at the root: close the screen (`sub_82D0AFA0(FE, 0x38, -1)`).
    Close,
}

/// The screen's navigation state (plain data: a host can snapshot it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChallengeMapState {
    /// The mode root (`progression` offline, `online` online).
    pub root: Option<EntryRef>,
    pub tab: usize,
    pub stack: Vec<EntryRef>,
    /// The selected row (`mgr+0xE5E8`, GetStuffMenuItems[1]); retail's helper index `+0xD8` follows it through the
    /// movie's OnMenuHilight round trip after every refresh, so one field gives the same outcome.
    pub hilight: usize,
    /// Each tab's row at the root (`mgr+0x118 + 4*tab`), keyed by the tab's id so a mod adding tabs keeps them.
    pub tab_rows: std::collections::BTreeMap<String, usize>,
    /// The map picture shown (`+0xEC`); None = the default map.
    pub picture: Option<String>,
    pub legend: bool,
}

/// The `world` map name retail treats as "use the default map" (string 0x821F89E4, b256).
pub const DEFAULT_PICTURE_NAME: &str = "map_portcarverton";

const REFRESH: [i32; 4] = [state::RELOAD_LIST, state::LIST_TITLE, state::SELECT_TAB, state::CURSOR];

impl ChallengeMapState {
    /// Screen open (`sub_826457E0`): the mode root, the preferred tab (ours: the first visible one; retail's
    /// "initial tab" Lua is not ported), and the open states (10, 90, 21, 33, 41).
    pub fn open(registry: &MenuRegistry, ctx: &MenuContext, root: &str) -> (Self, Vec<i32>) {
        let mut s = Self { root: Some(EntryRef::map(EntryKind::MapListing, root)), ..Default::default() };
        if let Some(i) = s.tabs(registry, ctx).iter().position(|t| t.id == "progression_locations") {
            s.tab = i;
        }
        (s, vec![state::RELOAD_LIST, state::DISTRICT_TITLE, state::LIST_PANEL, state::DEFAULT_MAP, state::LIST_TITLE_OFF])
    }

    /// The tabs (GetSuperCategories): the root's visible children.
    pub fn tabs(&self, registry: &MenuRegistry, ctx: &MenuContext) -> Vec<EntryRef> {
        let Some(root) = &self.root else { return Vec::new() };
        registry.data().map_children(root).into_iter().map(|n| n.entry.clone()).filter(|e| registry.visible(e, ctx)).collect()
    }

    /// The folder whose children are listed: the top of the stack, else the current tab.
    pub fn node(&self, registry: &MenuRegistry, ctx: &MenuContext) -> Option<EntryRef> {
        self.stack.last().cloned().or_else(|| self.tabs(registry, ctx).get(self.tab).cloned())
    }

    /// The current list (GetStuffMenuItems).
    pub fn rows(&self, registry: &MenuRegistry, ctx: &MenuContext) -> Vec<Row> {
        let Some(node) = self.node(registry, ctx) else { return Vec::new() };
        registry
            .data()
            .map_children(&node)
            .into_iter()
            .filter(|n| registry.visible(&n.entry, ctx))
            // A map_category header is never greyed (OnMenuGetItemEnabled `82646D18` defaults to enabled, b454); it is
            // a header clip the cursor skips.
            .map(|n| Row { entry: n.entry.clone(), label: n.label.clone(),
                enabled: n.entry.kind == EntryKind::MapCategory || registry.enabled(&n.entry, ctx) })
            .collect()
    }

    /// The list title (GetStuffTitleText): the folder's label.
    pub fn title(&self, registry: &MenuRegistry, ctx: &MenuContext) -> String {
        self.node(registry, ctx).and_then(|n| registry.data().map_node(&n).map(|m| m.label.clone())).unwrap_or_default()
    }

    /// `sub_82D0D100(mgr, index)`: the first real row (not a `map_category` header) at or after `index`, wrapping;
    /// unchanged when there is none. At the root it is also the tab's saved row.
    fn set_sel(&mut self, registry: &MenuRegistry, ctx: &MenuContext, index: usize) {
        let rows = self.rows(registry, ctx);
        let n = rows.len();
        if let Some(i) = (0..n).map(|k| (index + k) % n).find(|&i| rows[i].entry.kind != EntryKind::MapCategory) {
            self.hilight = i;
        }
        if self.stack.is_empty() {
            if let Some(tab) = self.tabs(registry, ctx).get(self.tab) {
                self.tab_rows.insert(tab.id.clone(), self.hilight);
            }
        }
    }

    /// The picture for the current list and row (`82649FF8`): the highlighted location's `world` map; folders and the
    /// root show the default map, as does `map_portcarverton`.
    pub fn picture_for(&self, registry: &MenuRegistry, ctx: &MenuContext) -> Option<String> {
        let rows = self.rows(registry, ctx);
        let row = rows.get(self.hilight).filter(|r| r.entry.kind == EntryKind::FeLocation)?;
        registry.data().map_pictures.get(&row.entry.id).filter(|n| n.as_str() != DEFAULT_PICTURE_NAME).cloned()
    }

    /// The map markers (`82649CD8` -> `8264A2A0`, b256 section 2): in a list of locations, one per location with marker
    /// data, in list order, the highlighted one last (`8268D898` draws it on top); other lists show none. Each is
    /// (icon texture name, map pixels from the centre, highlighted).
    pub fn markers(&self, registry: &MenuRegistry, ctx: &MenuContext) -> Vec<(String, [f32; 2], bool)> {
        let rows = self.rows(registry, ctx);
        let markers = &registry.data().map_markers;
        let mut out: Vec<(String, [f32; 2], bool)> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.entry.kind == EntryKind::FeLocation)
            .filter_map(|(i, r)| markers.get(&r.entry.id).map(|m| (m.icon.clone(), m.projected(), i == self.hilight)))
            .collect();
        out.sort_by_key(|(.., highlighted)| *highlighted);
        out
    }

    /// Re-reads the picture; SetState 34 (RefreshMapTexture) when it changed.
    fn refresh_picture(&mut self, registry: &MenuRegistry, ctx: &MenuContext, states: &mut Vec<i32>) {
        let picture = self.picture_for(registry, ctx);
        if picture != self.picture {
            self.picture = picture;
            states.push(state::MAP_TEXTURE);
        }
    }

    /// The refreshers after a push, pop or tab switch (b256 section 3), with the map picture.
    fn refresh(&mut self, registry: &MenuRegistry, ctx: &MenuContext) -> Action {
        let mut states = REFRESH.to_vec();
        self.refresh_picture(registry, ctx, &mut states);
        Action::Refresh(states)
    }

    /// OnMenuHilight (`826477D8`): +0xD8 = index and `D100(mgr, index)`; the map picture and cursor follow.
    pub fn hilight(&mut self, registry: &MenuRegistry, ctx: &MenuContext, index: usize) -> Vec<i32> {
        self.set_sel(registry, ctx, index);
        let mut states = Vec::new();
        self.refresh_picture(registry, ctx, &mut states);
        states.push(state::CURSOR);
        states
    }

    /// OnMenuSelect (`826475B8`): a folder with something in it is pushed; a leaf is selected; a greyed row or an
    /// empty folder does nothing.
    pub fn select(&mut self, registry: &MenuRegistry, ctx: &MenuContext, index: usize) -> Action {
        let rows = self.rows(registry, ctx);
        let Some(row) = rows.get(index) else { return Action::None };
        // A map_category header row does nothing (`826475B8`, research b454).
        if !row.enabled || row.entry.kind == EntryKind::MapCategory {
            return Action::None;
        }
        if row.entry.kind.is_map_folder() {
            if registry.data().map_children(&row.entry).is_empty() {
                return Action::None;
            }
            self.stack.push(row.entry.clone());
            self.set_sel(registry, ctx, 0);
            return self.refresh(registry, ctx);
        }
        Action::Select(row.entry.clone())
    }

    /// B (`82646B28`): hide the legend, else pop a folder, else close at the root. The pop (`sub_82D0D378`) selects
    /// the folder the player left in its parent's list (`D100` on its row; unchanged if it is not listed).
    pub fn back(&mut self, registry: &MenuRegistry, ctx: &MenuContext) -> Action {
        if self.legend {
            self.legend = false;
            return Action::Refresh(vec![state::LEGEND_OFF]);
        }
        if let Some(left) = self.stack.pop() {
            if let Some(i) = self.rows(registry, ctx).iter().position(|r| r.entry == left) {
                self.set_sel(registry, ctx, i);
            }
            return self.refresh(registry, ctx);
        }
        Action::Close
    }

    /// Y: show the legend (Y again does nothing; B hides it).
    pub fn legend_on(&mut self) -> Action {
        if self.legend {
            return Action::None;
        }
        self.legend = true;
        Action::Refresh(vec![state::LEGEND_ON])
    }

    /// Next / previous tab (triggers, d-pad or stick left / right): clamped, no wrap; the folder stack resets and the
    /// tab's saved row comes back (`sub_82D0C1C0`, then the load step `82D0D728`: `D100(mgr, mgr+0x118 + 4*tab)`;
    /// the first visit starts at row 0, ours: retail's initial slot values are not read, research b261).
    pub fn step_tab(&mut self, registry: &MenuRegistry, ctx: &MenuContext, forward: bool) -> Action {
        let count = self.tabs(registry, ctx).len();
        let next = if forward { (self.tab + 1).min(count.saturating_sub(1)) } else { self.tab.saturating_sub(1) };
        if next == self.tab || self.legend {
            return Action::None;
        }
        self.tab = next;
        self.stack.clear();
        let saved = self.tabs(registry, ctx).get(next).and_then(|t| self.tab_rows.get(&t.id).copied()).unwrap_or(0);
        self.set_sel(registry, ctx, saved);
        self.refresh(registry, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::{Handler, MapNode, MenuData, Owner};

    fn registry() -> MenuRegistry {
        let node = |kind, key: &str, parent: Option<(EntryKind, &str)>, order| MapNode {
            entry: EntryRef::map(kind, key),
            parent: parent.map(|(k, p)| EntryRef::map(k, p)),
            label: format!("L_{key}"),
            order,
        };
        use EntryKind::*;
        let data = MenuData {
            map_nodes: vec![
                node(MapListing, "progression", None, 0),
                node(MapListing, "progression_active", Some((MapListing, "progression")), 0),
                node(MapListing, "progression_locations", Some((MapListing, "progression")), 3),
                node(MapListing, "downtown", Some((MapListing, "progression_locations")), 0),
                node(MapListing, "empty", Some((MapListing, "progression_locations")), 1),
                node(MapListing, "uptown", Some((MapListing, "progression_locations")), 2),
                node(FeLocation, "uptown_spot", Some((MapListing, "uptown")), 0),
                node(FeLocation, "dist_downtown_aletown", Some((MapListing, "downtown")), 0),
                node(Challenge, "c1", Some((MapListing, "progression_active")), 0),
            ],
            ..Default::default()
        };
        let mut data = data;
        data.map_markers.insert(
            "dist_downtown_aletown".into(),
            crate::menus::MapMarker { icon: "mini_teleport".into(), point: [600.0, -100.0], bounds: [2050, -850, 1050, -1250, 512, 512] },
        );
        let mut r = MenuRegistry::new(data);
        let owner = Owner::Engine("teleport:dist_downtown_aletown".into());
        r.register_handler(EntryRef::map(FeLocation, "dist_downtown_aletown"), Handler { owner, select: true, highlight: false, callback: None });
        r
    }

    #[test]
    fn the_map_opens_on_locations_pushes_folders_and_selects_a_teleport() {
        let r = registry();
        let ctx = MenuContext::new("Career");
        let (mut s, open) = ChallengeMapState::open(&r, &ctx, "progression");
        assert_eq!(open, [10, 90, 21, 33, 41]);
        assert_eq!(s.tabs(&r, &ctx).iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["progression_active", "progression_locations"]);
        assert_eq!(s.tab, 1, "opens on the locations tab");
        let rows = s.rows(&r, &ctx);
        assert_eq!(rows.iter().map(|r| (r.entry.id.as_str(), r.enabled)).collect::<Vec<_>>(), [("downtown", true), ("empty", false), ("uptown", false)]);
        assert_eq!(s.title(&r, &ctx), "L_progression_locations");
        assert_eq!(s.select(&r, &ctx, 1), Action::None, "an empty, greyed folder does nothing");
        assert_eq!(s.select(&r, &ctx, 0), Action::Refresh(vec![10, 80, 100, 110]));
        assert_eq!(s.title(&r, &ctx), "L_downtown");
        // Downtown's one location: its marker, highlighted (b256 projection: x = 512/2 / 1450 * (600 - 600) * 0.99,
        // y = 512/2 / 1150 * (-100 - -100) * 0.95 = the map centre).
        assert_eq!(s.markers(&r, &ctx), vec![("mini_teleport".to_owned(), [0.0, 0.0], true)]);
        let off = crate::menus::MapMarker { icon: String::new(), point: [1325.0, 475.0], bounds: [2050, -850, 1050, -1250, 512, 512] };
        let [x, y] = off.projected();
        assert!((x - 256.0 / 1450.0 * 725.0 * 0.99).abs() < 1e-3 && (y - 256.0 / 1150.0 * 575.0 * 0.95).abs() < 1e-3, "{x} {y}");
        assert_eq!(s.select(&r, &ctx, 0), Action::Select(EntryRef::map(EntryKind::FeLocation, "dist_downtown_aletown")));
        assert_eq!(s.hilight(&r, &ctx, 0), [110]);
        // Back pops to the folder's row in its parent (`sub_82D0D378`), then closes at the root.
        assert!(matches!(s.back(&r, &ctx), Action::Refresh(_)));
        assert_eq!(s.hilight, 0);
        s.hilight(&r, &ctx, 2);
        s.stack.push(EntryRef::map(EntryKind::MapListing, "uptown"));
        assert!(matches!(s.back(&r, &ctx), Action::Refresh(_)));
        assert_eq!(s.hilight, 2, "back on the Uptown row");
        // Tabs clamp, no wrap; the challenges tab's challenge has no handler, so it is greyed. Each tab comes back on
        // its saved row (`mgr+0x118 + 4*tab`).
        assert_eq!(s.step_tab(&r, &ctx, true), Action::None);
        assert!(matches!(s.step_tab(&r, &ctx, false), Action::Refresh(_)));
        assert_eq!(s.hilight, 0, "a first visit starts at row 0");
        assert_eq!(s.rows(&r, &ctx)[0].enabled, false);
        assert_eq!(s.step_tab(&r, &ctx, false), Action::None);
        assert!(matches!(s.step_tab(&r, &ctx, true), Action::Refresh(_)));
        assert_eq!(s.hilight, 2, "the locations tab's saved row");
        assert!(matches!(s.step_tab(&r, &ctx, false), Action::Refresh(_)));
        // The legend: Y shows it, B hides it before anything else.
        assert!(matches!(s.legend_on(), Action::Refresh(_)));
        assert_eq!(s.legend_on(), Action::None);
        assert_eq!(s.back(&r, &ctx), Action::Refresh(vec![130]));
        assert_eq!(s.back(&r, &ctx), Action::Close);
    }
}
