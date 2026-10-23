//! The Trick Guide screen (pause menu Learn > Trick Guide; FE screen 14, template 29 TrickTutorial, movie
//! `screens/tricks/trickguide`; doc 31 "Trick Guide"). Port of retail's screen-14 object (TU3 vtable 0x823038B8):
//! a three-level list (categories, subcategories, tricks) over the trick records, the picker callbacks and the
//! FETrickTutorial natives the movie reads. Research `.local/research/menus-m6/trick-guide-open.md` and
//! `trick-guide-gestures.md` (main re-read the handlers named below).
//!
//! The data is a [`TrickGuide`]: setup's records by Attrib key (scoring_trickcategory / tricksubcategory /
//! scoring_trick), the category order and the slot name tables. Every part is plain data, so a mod can add or
//! change entries by key; the navigation reads nothing else.
use std::collections::BTreeMap;

/// A record's 64-bit Attrib key: the stable id of a category, subcategory or trick.
pub type Key = u64;

/// One input slot (`Sk8::FE::tTrickGuideInputSlot`, 12 bytes): ids into [`TrickGuide::gestures`] and
/// [`TrickGuide::buttons`], and the connector to the next slot (1 Plus, 2 Arrow, 3 Or, else none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub gesture: u32,
    pub button: u32,
    pub connector: u32,
}

/// Retail's 4 slots per screen (S+144, 12 bytes each; the movie's mMAX_GESTURES).
pub const SLOTS: usize = 4;

/// Connector strings the movie compares (GetMenuItemGesturesButtons 825AE968).
pub const CONNECTORS: [&str; 4] = ["", "Plus", "Arrow", "Or"];

/// Gesture ids GetMenuItemGesturesButtons shows as "" (58 and 59; the data uses 59).
const BLANK_GESTURES: [u32; 2] = [58, 59];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// Name (string id): category / subcategory field 0x7296FFD6010D73EF, trick label 0x843613E915014627.
    pub label: String,
    /// Description (string id): 0x273DF7D4EE7A4C1C, trick 0x4663ADFED2CD5EF7.
    pub description: String,
    /// Input slots (array 0x03ACC9F200807B87).
    pub slots: Vec<Slot>,
    /// The demo animation (`trickguide_*`, 0x3B71EA27D53D7C4F).
    pub animation: String,
    /// The demo camera track: the Int16 of the `trickbook_camera_tracks` record that RefSpec 0x3D3A79F16DCB54BB
    /// names (sub_826188B8); no ref reads 0 (the zero block 0x830D0850).
    pub camera_track: i16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Category {
    pub entry: Entry,
    /// Its subcategories in order (RefSpec array 0x57035D975F71D506).
    pub subcategories: Vec<Key>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subcategory {
    pub entry: Entry,
    /// Bool 0xA990472B974734C3 (default true): the entry opens a trick list. Leaf entries (No Comply, Sprint, ...)
    /// show their own slots instead.
    pub has_tricks: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trick {
    pub entry: Entry,
    /// The subcategory it lists under (RefSpec 0x06513CC28A532524).
    pub subcategory: Option<Key>,
    /// Sort order in its list (Int32 0xF9E5D2F69066E925, layout offset 128).
    pub order: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrickGuide {
    /// The top-level list (TU3 0x830274B0, extracted at setup).
    pub categories: Vec<Key>,
    pub category: BTreeMap<Key, Category>,
    pub subcategory: BTreeMap<Key, Subcategory>,
    /// Tricks in record order (the order retail's class iteration visits them; ties in [`Trick::order`] keep it).
    pub tricks: Vec<(Key, Trick)>,
    /// Gesture animation names by slot gesture id (TU3 0x830273C8).
    pub gestures: Vec<String>,
    /// Button names by slot button id (TU3 0x83027350).
    pub buttons: Vec<String>,
}

/// Which list an entry belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Category,
    Subcategory,
    Trick,
}

/// A change to one entry (a mod's `sdk.menus.trick_guide`): unset fields keep the entry's value; a new key creates
/// the entry. `parent` is the category a subcategory lists under (appended) or the subcategory a trick lists under;
/// `position` places a category in the top-level list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryPatch {
    pub label: Option<String>,
    pub description: Option<String>,
    pub slots: Option<Vec<Slot>>,
    pub animation: Option<String>,
    pub camera_track: Option<i16>,
    pub parent: Option<Key>,
    pub order: Option<i32>,
    pub has_tricks: Option<bool>,
    pub position: Option<usize>,
    /// Remove the entry from its list (it stays reachable by key for slots).
    pub hidden: bool,
}

impl TrickGuide {
    fn trick(&self, key: Key) -> Option<&Trick> {
        self.tricks.iter().find(|(k, _)| *k == key).map(|(_, t)| t)
    }

    /// Applies one patch (mods' patches in their order over the retail data).
    pub fn apply(&mut self, kind: EntryKind, key: Key, patch: &EntryPatch) {
        fn edit(entry: &mut Entry, patch: &EntryPatch) {
            if let Some(v) = &patch.label {
                entry.label = v.clone();
            }
            if let Some(v) = &patch.description {
                entry.description = v.clone();
            }
            if let Some(v) = &patch.slots {
                entry.slots = v.iter().take(SLOTS).copied().collect();
            }
            if let Some(v) = &patch.animation {
                entry.animation = v.clone();
            }
            if let Some(v) = patch.camera_track {
                entry.camera_track = v;
            }
        }
        match kind {
            EntryKind::Category => {
                edit(&mut self.category.entry(key).or_default().entry, patch);
                self.categories.retain(|k| *k != key);
                if !patch.hidden {
                    let at = patch.position.unwrap_or(self.categories.len()).min(self.categories.len());
                    self.categories.insert(at, key);
                }
            }
            EntryKind::Subcategory => {
                let sub = self.subcategory.entry(key).or_insert_with(|| Subcategory { has_tricks: true, ..Default::default() });
                edit(&mut sub.entry, patch);
                if let Some(v) = patch.has_tricks {
                    sub.has_tricks = v;
                }
                if patch.hidden {
                    for c in self.category.values_mut() {
                        c.subcategories.retain(|k| *k != key);
                    }
                } else if let Some(parent) = patch.parent {
                    for c in self.category.values_mut() {
                        c.subcategories.retain(|k| *k != key);
                    }
                    if let Some(c) = self.category.get_mut(&parent) {
                        c.subcategories.push(key);
                    }
                }
            }
            EntryKind::Trick => {
                let at = match self.tricks.iter().position(|(k, _)| *k == key) {
                    Some(i) => i,
                    None => {
                        self.tricks.push((key, Trick::default()));
                        self.tricks.len() - 1
                    }
                };
                let trick = &mut self.tricks[at].1;
                edit(&mut trick.entry, patch);
                if let Some(v) = patch.parent {
                    trick.subcategory = Some(v);
                }
                if let Some(v) = patch.order {
                    trick.order = v;
                }
                // Listed tricks need a demo animation (the list fill skips the rest).
                if patch.hidden {
                    trick.entry.animation.clear();
                }
            }
        }
    }
}

/// What the screen asks the movie to do next (the facade's SetScreenState, sub_8263F320 index 10).
pub mod movie_state {
    /// Rebuild the list after going up a level (back, vt+40).
    pub const BACK: i32 = 0;
    /// Rebuild the list (select, vt+64).
    pub const REBUILD: i32 = 1;
    /// Hide the gesture row (highlight without slots).
    pub const NO_GESTURES: i32 = 10;
    /// Show the slots (sub_82619D38).
    pub const GESTURES: i32 = 11;
}

/// The player facts the button remap reads (sub_82619FC0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stance {
    /// CAS byte +120 of the player's skater record; HUDComponents.PlayerStance_IsRegular returns it (826635A0).
    pub regular: bool,
    /// `[[sub_824AD2F0()]+224]+20`. NOT RETAIL YET: meaning not decoded (a controls option); false here.
    pub alternate: bool,
    /// `[[sub_824AD2F0()]+260]+37`: zero makes a remapped X read B, else A. NOT RETAIL YET: meaning not decoded;
    /// set here (the remap then undoes the movie's own A / X swap).
    pub x_reads_a: bool,
}

impl Default for Stance {
    fn default() -> Self {
        Self { regular: true, alternate: false, x_reads_a: true }
    }
}

/// Button ids sub_82619FC0 remaps.
const BUTTON_A: u32 = 3;
const BUTTON_B: u32 = 4;
const BUTTON_X: u32 = 6;
const BUTTON_RT: u32 = 7;
const BUTTON_LT: u32 = 8;

/// sub_82619FC0: when exactly one of the two stance flags is set, the triggers swap, A shows X and X shows A or B.
pub fn remap_button(button: u32, stance: Stance) -> u32 {
    if stance.regular == stance.alternate {
        return button;
    }
    match button {
        BUTTON_LT => BUTTON_RT,
        BUTTON_RT => BUTTON_LT,
        BUTTON_X if stance.x_reads_a => BUTTON_A,
        BUTTON_X => BUTTON_B,
        BUTTON_A => BUTTON_X,
        other => other,
    }
}

/// The screen-14 object's navigation fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrickGuideState {
    /// Current level 0..=2 (S+112).
    pub level: usize,
    /// Highlighted row (S+116; GetCurrentMenuItem).
    pub index: usize,
    /// Row to restore on the way back up: level 0 (S+120) and level 1 (S+124).
    pub saved: [usize; 2],
    /// The key list per level (S+64, S+80, S+96).
    pub lists: [Vec<Key>; 3],
    /// S+128: the slots come from the subcategory records (set when the first subcategory of the opened list has no
    /// tricks; cleared on the way back up).
    pub slots_from_subcategory: bool,
    /// The highlighted entry's slots, demo animation and camera track (S+144, S+204, S+200; sub_82619D38).
    pub slots: [Slot; SLOTS],
    pub animation: String,
    pub camera_track: i16,
}

/// Back's outcome (vt+40, action 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Back {
    /// Level 0: fade out and close the screen.
    Close,
    /// Went up a level: SetScreenState(state).
    Up(i32),
}

impl TrickGuideState {
    /// Screen init (0x82618A40): level 0, row 0, the category list.
    pub fn open(guide: &TrickGuide) -> Self {
        let mut state = Self::default();
        state.fill(guide, 0, 0);
        state
    }

    /// The list fill (sub_82619368) for `level`, from row `index` of the level above. Clears the lists below.
    fn fill(&mut self, guide: &TrickGuide, mut level: usize, index: usize) {
        let mut index = index;
        match level {
            0 => self.lists[0] = guide.categories.clone(),
            _ => {
                if level == 1 {
                    let subs = guide
                        .categories
                        .get(index)
                        .and_then(|k| guide.category.get(k))
                        .map(|c| c.subcategories.clone())
                        .unwrap_or_default();
                    // S+128 = !has_tricks of the list's first entry (a missing record reads retail's zero default).
                    if let Some(first) = subs.first() {
                        self.slots_from_subcategory = !guide.subcategory.get(first).is_some_and(|s| s.has_tricks);
                    }
                    let count = subs.len();
                    self.lists[1] = subs;
                    if count >= 2 {
                        self.clear_below(1);
                        return;
                    }
                    // A level with fewer than 2 entries is skipped: straight to the tricks of its first entry.
                    self.level += 1;
                    level = 2;
                    index = 0;
                }
                let Some(&sub) = self.lists[1].get(index) else {
                    self.lists[2].clear();
                    self.clear_below(level);
                    return;
                };
                // Tricks with a demo animation under this subcategory, sorted by their order (stable).
                let mut tricks: Vec<(i32, Key)> = guide
                    .tricks
                    .iter()
                    .filter(|(_, t)| !t.entry.animation.is_empty() && t.subcategory == Some(sub))
                    .map(|(k, t)| (t.order, *k))
                    .collect();
                tricks.sort_by_key(|(order, _)| *order);
                self.lists[2] = tricks.into_iter().map(|(_, k)| k).collect();
            }
        }
        self.clear_below(level);
    }

    fn clear_below(&mut self, level: usize) {
        for list in self.lists.iter_mut().skip(level + 1) {
            list.clear();
        }
    }

    fn restore(&mut self) {
        self.index = match self.level {
            0 => self.saved[0],
            1 => self.saved[1],
            _ => self.index,
        };
    }

    /// Game.OnMenuSelect (vt+64, 0x82618D98): open the row's list; an empty list goes back. Returns the movie state.
    pub fn select(&mut self, guide: &TrickGuide, index: usize) -> i32 {
        match self.level {
            0 | 1 => {
                self.saved[self.level] = self.index;
                self.level += 1;
                self.index = 0;
                self.fill(guide, self.level, index);
            }
            _ => {}
        }
        if self.lists[self.level].is_empty() && self.level > 0 {
            self.level -= 1;
            self.restore();
        }
        movie_state::REBUILD
    }

    /// Back (vt+40, action 3): close at level 0, else up one level (twice past a level with fewer than 2 entries).
    pub fn back(&mut self) -> Back {
        if self.level == 0 {
            return Back::Close;
        }
        self.lists[self.level].clear();
        self.level -= 1;
        self.restore();
        // NOT RETAIL (guard): retail would step below level 0 if the category list had fewer than 2 entries.
        if self.lists[self.level].len() < 2 && self.level > 0 {
            self.lists[self.level].clear();
            self.level -= 1;
            self.restore();
        }
        self.slots_from_subcategory = false;
        Back::Up(movie_state::BACK)
    }

    /// Game.OnMenuHilight (vt+68, 0x82618E98): the row; then the slots of its record, or no gesture row on the upper
    /// levels. Returns the movie state.
    pub fn hilight(&mut self, guide: &TrickGuide, index: usize) -> i32 {
        self.index = index;
        if !self.slots_from_subcategory && self.level != 2 {
            return movie_state::NO_GESTURES;
        }
        // sub_82619D38: the key from this level's list, as a trick or (S+128) as a subcategory.
        self.slots = [Slot::default(); SLOTS];
        self.animation.clear();
        let key = self.lists[self.level].get(index).copied();
        let entry = key.and_then(|k| {
            if self.slots_from_subcategory {
                guide.subcategory.get(&k).map(|s| &s.entry)
            } else {
                guide.trick(k).map(|t| &t.entry)
            }
        });
        if let Some(entry) = entry {
            for (slot, value) in self.slots.iter_mut().zip(&entry.slots) {
                *slot = *value;
            }
            self.animation = entry.animation.clone();
            self.camera_track = entry.camera_track;
        }
        movie_state::GESTURES
    }

    /// The input handler's return (vt+40): A on a row without a list is consumed (the movie never sees it).
    pub fn consumes_select(&self, guide: &TrickGuide) -> bool {
        !self.has_submenu(guide)
    }

    fn key(&self) -> Option<Key> {
        self.lists[self.level].get(self.index).copied()
    }

    /// sub_826192A0: categories always open a list, subcategories by their flag, tricks never.
    pub fn has_submenu(&self, guide: &TrickGuide) -> bool {
        match self.level {
            0 => true,
            1 => self.key().and_then(|k| guide.subcategory.get(&k)).is_some_and(|s| s.has_tricks),
            _ => false,
        }
    }

    fn entry<'a>(&self, guide: &'a TrickGuide, level: usize, key: Key) -> Option<&'a Entry> {
        match level {
            0 => guide.category.get(&key).map(|c| &c.entry),
            1 => guide.subcategory.get(&key).map(|s| &s.entry),
            _ => guide.trick(key).map(|t| &t.entry),
        }
    }

    /// Game.GetMenuItems (825A52A8 over vt+80 / vt+84, 0x826190D8 / 0x82619100): the current list's names.
    pub fn items(&self, guide: &TrickGuide) -> Vec<String> {
        self.lists[self.level]
            .iter()
            .map(|k| self.entry(guide, self.level, *k).map(|e| e.label.clone()).unwrap_or_default())
            .collect()
    }

    /// The highlighted row's description (vt+88, 0x826191B0).
    pub fn description(&self, guide: &TrickGuide) -> String {
        self.key().and_then(|k| self.entry(guide, self.level, k)).map(|e| e.description.clone()).unwrap_or_default()
    }

    /// FETrickTutorial.GetMenuItemGesturesButtons (825AE968): the used-slot count (gesture or button set), then
    /// gesture name, button name (remapped) and connector per slot.
    pub fn gestures_buttons(&self, guide: &TrickGuide, stance: Stance) -> (usize, Vec<[String; 3]>) {
        let used = self.slots.iter().filter(|s| s.gesture != 0 || s.button != 0).count();
        let name = |table: &[String], i: u32| table.get(i as usize).cloned().unwrap_or_default();
        let slots = self
            .slots
            .iter()
            .map(|s| {
                let gesture = if BLANK_GESTURES.contains(&s.gesture) { 0 } else { s.gesture };
                [
                    name(&guide.gestures, gesture),
                    name(&guide.buttons, remap_button(s.button, stance)),
                    CONNECTORS.get(s.connector as usize).copied().unwrap_or("").to_owned(),
                ]
            })
            .collect();
        (used, slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str) -> Entry {
        Entry { label: label.into(), description: format!("{label}_DESC"), ..Default::default() }
    }

    /// Two categories: Flip (two subcategories with tricks) and Manuals (one subcategory, skipped); Other (two leaf
    /// subcategories with their own slots).
    pub(crate) fn guide() -> TrickGuide {
        let mut g = TrickGuide { gestures: vec!["".into(), "ollie".into()], buttons: vec!["".into(), "Back".into(), "Start".into(), "A".into(), "B".into(), "Y".into(), "X".into(), "R_Trigger".into(), "L_Trigger".into()], ..Default::default() };
        g.categories = vec![1, 2, 3];
        g.category.insert(1, Category { entry: entry("FLIP"), subcategories: vec![10, 11] });
        g.category.insert(2, Category { entry: entry("MANUALS"), subcategories: vec![20] });
        g.category.insert(3, Category { entry: entry("OTHER"), subcategories: vec![30, 31] });
        for (k, name, has_tricks) in [(10, "OLLIE_FLIPS", true), (11, "NOLLIE_FLIPS", true), (20, "MANUAL", true), (30, "NOCOMPLY", false), (31, "SPRINT", false)] {
            let mut e = entry(name);
            if !has_tricks {
                e.slots = vec![Slot { gesture: 0, button: 3, connector: 2 }, Slot { gesture: 1, button: 0, connector: 0 }];
            }
            g.subcategory.insert(k, Subcategory { entry: e, has_tricks });
        }
        let trick = |name: &str, sub: Key, order: i32, animation: &str| Trick {
            entry: Entry { animation: animation.into(), slots: vec![Slot { gesture: 1, button: 0, connector: 0 }], ..entry(name) },
            subcategory: Some(sub),
            order,
        };
        g.tricks = vec![
            (100, trick("HARDFLIP", 10, 11, "trickguide_hardflip")),
            (101, trick("KICKFLIP", 10, 2, "trickguide_kickflip")),
            (102, trick("HIDDEN", 10, 1, "")),
            (103, trick("NOLLIE_KICKFLIP", 11, 1, "a")),
            (200, trick("MANUAL", 20, 0, "m")),
            (201, trick("NOSE_MANUAL", 20, 1, "n")),
        ];
        g
    }

    #[test]
    fn select_and_back_walk_the_three_levels() {
        let g = guide();
        let mut s = TrickGuideState::open(&g);
        assert_eq!(s.items(&g), ["FLIP", "MANUALS", "OTHER"]);
        assert!(s.has_submenu(&g));
        assert_eq!(s.hilight(&g, 0), movie_state::NO_GESTURES);
        assert_eq!(s.select(&g, 0), movie_state::REBUILD);
        assert_eq!((s.level, s.index), (1, 0));
        assert_eq!(s.items(&g), ["OLLIE_FLIPS", "NOLLIE_FLIPS"]);
        s.select(&g, 0);
        // Sorted by order; no animation = not listed.
        assert_eq!(s.items(&g), ["KICKFLIP", "HARDFLIP"]);
        assert!(!s.has_submenu(&g) && s.consumes_select(&g));
        assert_eq!(s.hilight(&g, 1), movie_state::GESTURES);
        assert_eq!((s.description(&g), s.animation.as_str()), ("HARDFLIP_DESC".to_string(), "trickguide_hardflip"));
        assert_eq!(s.back(), Back::Up(movie_state::BACK));
        assert_eq!((s.level, s.items(&g).len()), (1, 2));
        assert_eq!(s.back(), Back::Up(movie_state::BACK));
        assert_eq!(s.level, 0);
        assert_eq!(s.back(), Back::Close);
    }

    #[test]
    fn a_single_subcategory_is_skipped_both_ways() {
        let g = guide();
        let mut s = TrickGuideState::open(&g);
        s.hilight(&g, 1);
        s.select(&g, 1);
        assert_eq!((s.level, s.items(&g)), (2, vec!["MANUAL".to_string(), "NOSE_MANUAL".into()]));
        // Back skips the one-entry level and restores the category row.
        s.back();
        assert_eq!((s.level, s.index), (0, 1));
    }

    #[test]
    fn leaf_subcategories_show_their_own_slots() {
        let g = guide();
        let mut s = TrickGuideState::open(&g);
        s.select(&g, 2);
        assert!(s.slots_from_subcategory && !s.has_submenu(&g));
        assert_eq!(s.hilight(&g, 0), movie_state::GESTURES);
        let (used, slots) = s.gestures_buttons(&g, Stance { regular: false, alternate: false, x_reads_a: true });
        assert_eq!(used, 2);
        assert_eq!(slots[0], ["".to_string(), "A".into(), "Arrow".into()]);
        assert_eq!(slots[1], ["ollie".to_string(), "".into(), "".into()]);
        // Regular stance: A shows X (the movie swaps it back).
        let (_, slots) = s.gestures_buttons(&g, Stance::default());
        assert_eq!(slots[0][1], "X");
        s.back();
        assert!(!s.slots_from_subcategory);
    }

    #[test]
    fn patches_add_change_and_hide_entries() {
        let mut g = guide();
        let label = |l: &str| EntryPatch { label: Some(l.into()), ..Default::default() };
        g.apply(EntryKind::Category, 9, &EntryPatch { position: Some(0), ..label("MOD") });
        g.apply(EntryKind::Subcategory, 90, &EntryPatch { parent: Some(9), ..label("MOD_SUB") });
        g.apply(EntryKind::Subcategory, 91, &EntryPatch { parent: Some(9), ..label("MOD_SUB2") });
        g.apply(EntryKind::Trick, 900, &EntryPatch { parent: Some(90), animation: Some("a".into()), ..label("MOD_TRICK") });
        g.apply(EntryKind::Trick, 101, &label("KICKFLIP_RENAMED"));
        g.apply(EntryKind::Category, 2, &EntryPatch { hidden: true, ..Default::default() });
        let mut s = TrickGuideState::open(&g);
        assert_eq!(s.items(&g), ["MOD", "FLIP", "OTHER"]);
        s.select(&g, 0);
        s.select(&g, 0);
        assert_eq!(s.items(&g), ["MOD_TRICK"]);
        let mut s = TrickGuideState::open(&g);
        s.select(&g, 1);
        s.select(&g, 0);
        assert_eq!(s.items(&g), ["KICKFLIP_RENAMED", "HARDFLIP"]);
    }

    #[test]
    fn remap_follows_sub_82619fc0() {
        let on = Stance { regular: true, alternate: false, x_reads_a: false };
        assert_eq!((remap_button(8, on), remap_button(7, on), remap_button(3, on), remap_button(6, on)), (7, 8, 6, 4));
        assert_eq!(remap_button(6, Stance { x_reads_a: true, ..on }), 3);
        let both = Stance { regular: true, alternate: true, x_reads_a: true };
        assert_eq!(remap_button(8, both), 8);
    }
}
