//! The profile unlock store: retail's `UnlockablesBackEnd` (global `[0x83067074]`, 0x2F10 bytes, built by 825039F0;
//! TU3, research of 2 Nov 2026, docs/customization-survey.md). Every lock in the game asks it: the CAC shops (kind 5
//! clothing unlocks, kind 6 sponsor inventories), characters, gestures, skate.Park sets, worlds, challenges, DLC.
//!
//! - Two tables of at most 500 entries `{kind, FNV-1 hash of the name, new}`: the granted table (+0x28, count
//!   +0x1798; saved with the profile) and the content table (+0x179C, count +0x2F0C; filled at boot from the
//!   `data\unlocks\*.unlock` files, `KIND,name` lines, by 82505220, never saved).
//! - Grant 82503C30: skipped when that table already holds the pair; a granted entry starts new (1), a content entry
//!   not new (0). 82503BD8 hashes the name and grants only kinds below 13.
//! - Has 82503AE0: always true with the unlock-all byte (+0x24); else the granted table, and the content table too when
//!   asked (`both`, the CAC's DLC and sponsor checks 825F33A0 / 825F35A8).
//! - New 82503B60 (the CAC's "new" mark, 825F4160): the entry's byte, granted table first; never new with unlock-all.
//! - Closing the CAC (82603358) clears the new byte of every kind 5 and kind 6 entry.
//! - A fresh profile (825046B0) grants 7 characters and the 37 gestures, then clears their new bytes.
//! - Save 82505150 writes the granted table (count, then kind, hash, new per entry); load 82505000 grants each saved
//!   entry again (duplicates skipped) and restores its new byte. The unlock-all byte is not saved.

/// Retail's unlock kinds, in index order (string table 0x8302CC7C).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnlockKind {
    Locations = 0,
    Characters = 1,
    Replay = 2,
    Eggs = 3,
    Gestures = 4,
    ClothingUnlock = 5,
    ClothingSponsor = 6,
    SkateparkSet = 7,
    Entitlements = 8,
    Worlds = 9,
    Challenges = 10,
    Movie = 11,
    DlcProduct = 12,
}

impl UnlockKind {
    pub const ALL: [UnlockKind; 13] = [
        Self::Locations,
        Self::Characters,
        Self::Replay,
        Self::Eggs,
        Self::Gestures,
        Self::ClothingUnlock,
        Self::ClothingSponsor,
        Self::SkateparkSet,
        Self::Entitlements,
        Self::Worlds,
        Self::Challenges,
        Self::Movie,
        Self::DlcProduct,
    ];

    /// Retail's index; 13 and above are refused (82503BD8, 82505000).
    pub fn from_index(i: u32) -> Option<Self> {
        Self::ALL.get(i as usize).copied()
    }

    /// The name in retail's string table (also the first field of a `.unlock` line).
    pub fn name(self) -> &'static str {
        [
            "LOCATIONS",
            "CHARACTERS",
            "REPLAY",
            "EGGS",
            "GESTURES",
            "CLOTHING_UNLOCK",
            "CLOTHING_SPONSOR",
            "SKATEPARK_SET",
            "ENTITLEMENTS",
            "WORLDS",
            "CHALLENGES",
            "MOVIE",
            "DLC_PRODUCT",
        ][self as usize]
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The store kinds of `extras`' grants (`entitlement`, `sponsor`, `location`). Its `challenge` grants are not for
    /// this store: retail hands them to the progression manager (TheSim+0xE0 vt+0x14).
    pub fn from_extras(kind: &str) -> Option<Self> {
        match kind {
            "entitlement" => Some(Self::Entitlements),
            "sponsor" => Some(Self::ClothingSponsor),
            "location" => Some(Self::Locations),
            _ => None,
        }
    }
}

/// Retail's name hash: 32-bit FNV-1 (multiply, then xor) over the name's bytes as written.
pub fn hash(name: &str) -> u32 {
    name.bytes().fold(0x811C_9DC5u32, |h, b| h.wrapping_mul(0x0100_0193) ^ u32::from(b))
}

/// Entries per table (both tables are 500 x 12 bytes).
pub const CAPACITY: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: UnlockKind,
    pub hash: u32,
    /// Unlocked since the player last left the CAC (kinds 5 and 6) or never shown.
    pub new: bool,
}

/// Which table a grant goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    /// Saved with the profile.
    Granted,
    /// Content packs, from the `.unlock` files at boot; not saved.
    Content,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnlockStore {
    pub granted: Vec<Entry>,
    pub content: Vec<Entry>,
    /// Retail's byte +0x24: every check passes and nothing is new. Not saved. Ours: the debug unlock-all setting.
    pub unlock_all: bool,
}

/// What a fresh profile is granted (825046B0), kind 1 then kind 4, in retail's order.
pub const FRESH_CHARACTERS: [&str; 7] = ["sammy", "mike", "slappy", "shingo", "cuz", "seb", "freeway"];
pub const FRESH_GESTURES: [&str; 37] = [
    "gesture_airguitar",
    "gesture_airplane",
    "gesture_boxing",
    "gesture_bruce",
    "gesture_checktime",
    "gesture_devil",
    "gesture_doublegun",
    "gesture_dunno",
    "gesture_fingerwag",
    "gesture_fists",
    "gesture_flex",
    "gesture_fliptable",
    "gesture_fonz",
    "gesture_freedom",
    "gesture_fu",
    "gesture_getaway",
    "gesture_getouttahere",
    "gesture_handcuffs",
    "gesture_pumphi",
    "gesture_pumplo",
    "gesture_prewind",
    "gesture_peace",
    "gesture_fingerpoint",
    "gesture_raisetheroof",
    "gesture_shaka",
    "gesture_shrug",
    "gesture_sky",
    "gesture_snap",
    "gesture_soularch",
    "gesture_spock",
    "gesture_surfsup",
    "gesture_swinghigh",
    "gesture_swinglow",
    "gesture_throwarms",
    "gesture_thumbdown",
    "gesture_wings",
    "gesture_yardsale",
];

impl UnlockStore {
    /// A fresh profile's store (825046B0): the default characters and gestures, none of them new.
    pub fn fresh_profile() -> Self {
        let mut store = Self::default();
        for name in FRESH_CHARACTERS {
            store.grant(UnlockKind::Characters, name);
        }
        for name in FRESH_GESTURES {
            store.grant(UnlockKind::Gestures, name);
        }
        for e in &mut store.granted {
            e.new = false;
        }
        store
    }

    fn table(&self, table: Table) -> &Vec<Entry> {
        match table {
            Table::Granted => &self.granted,
            Table::Content => &self.content,
        }
    }

    fn find(&self, table: Table, kind: UnlockKind, hash: u32) -> Option<&Entry> {
        self.table(table).iter().find(|e| e.kind == kind && e.hash == hash)
    }

    /// Grant by name into the saved table (82503BD8). False when it was already granted.
    pub fn grant(&mut self, kind: UnlockKind, name: &str) -> bool {
        self.grant_hash(Table::Granted, kind, hash(name))
    }

    /// Grant by hash (82503C30). False when the table already holds the pair, or (ours) when it is full: retail writes
    /// past the end.
    pub fn grant_hash(&mut self, table: Table, kind: UnlockKind, hash: u32) -> bool {
        if self.find(table, kind, hash).is_some() {
            return false;
        }
        let entries = match table {
            Table::Granted => &mut self.granted,
            Table::Content => &mut self.content,
        };
        if entries.len() >= CAPACITY {
            return false;
        }
        entries.push(Entry { kind, hash, new: table == Table::Granted });
        true
    }

    /// Is it unlocked (82503AE0)? `both` also looks in the content table.
    pub fn has(&self, kind: UnlockKind, name: &str, both: bool) -> bool {
        self.has_hash(kind, hash(name), both)
    }

    pub fn has_hash(&self, kind: UnlockKind, hash: u32, both: bool) -> bool {
        self.unlock_all
            || self.find(Table::Granted, kind, hash).is_some()
            || (both && self.find(Table::Content, kind, hash).is_some())
    }

    /// Is it new (82503B60)?
    pub fn is_new(&self, kind: UnlockKind, name: &str) -> bool {
        let h = hash(name);
        !self.unlock_all
            && self.find(Table::Granted, kind, h).or_else(|| self.find(Table::Content, kind, h)).is_some_and(|e| e.new)
    }

    /// Clear the new byte of every granted entry of `kind` (82603358 does kinds 5 and 6 when the CAC closes).
    pub fn clear_new(&mut self, kind: UnlockKind) {
        for e in self.granted.iter_mut().filter(|e| e.kind == kind) {
            e.new = false;
        }
    }

    /// Read a content pack's `.unlock` file (82505220): one `KIND,name` per line; unknown kinds are skipped.
    pub fn grant_content_file(&mut self, text: &str) {
        for line in text.lines() {
            if let Some((kind, name)) = line.trim().split_once(',') {
                if let Some(kind) = UnlockKind::from_name(kind) {
                    self.grant_hash(Table::Content, kind, hash(name));
                }
            }
        }
    }

    /// Load saved entries on top of this store (82505000): each is granted again, then its new byte restored.
    pub fn load(&mut self, saved: &[Entry]) {
        for s in saved {
            self.grant_hash(Table::Granted, s.kind, s.hash);
            if let Some(e) = self.granted.iter_mut().find(|e| e.kind == s.kind && e.hash == s.hash) {
                e.new = s.new;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_fnv1() {
        // FNV-1 32 test vectors.
        assert_eq!(hash(""), 0x811C_9DC5);
        assert_eq!(hash("a"), 0x050C_5D7E);
        assert_eq!(hash("foobar"), 0x31F0_B262);
    }

    #[test]
    fn kinds_round_trip() {
        for k in UnlockKind::ALL {
            assert_eq!(UnlockKind::from_index(k as u32), Some(k));
            assert_eq!(UnlockKind::from_name(k.name()), Some(k));
        }
        assert_eq!(UnlockKind::from_index(13), None);
        assert_eq!(UnlockKind::from_extras("sponsor"), Some(UnlockKind::ClothingSponsor));
        assert_eq!(UnlockKind::from_extras("challenge"), None);
    }

    #[test]
    fn fresh_profile_has_the_defaults_none_new() {
        let s = UnlockStore::fresh_profile();
        assert_eq!(s.granted.len(), 44);
        assert!(s.has(UnlockKind::Characters, "slappy", false));
        assert!(s.has(UnlockKind::Gestures, "gesture_yardsale", false));
        assert!(!s.is_new(UnlockKind::Gestures, "gesture_yardsale"));
        assert!(!s.has(UnlockKind::ClothingUnlock, "cac_unlock_bs_group1", true));
    }

    #[test]
    fn grants_are_new_once_and_cleared_on_cac_exit() {
        let mut s = UnlockStore::fresh_profile();
        assert!(s.grant(UnlockKind::ClothingUnlock, "cac_unlock_bs_group1"));
        assert!(!s.grant(UnlockKind::ClothingUnlock, "cac_unlock_bs_group1"));
        assert!(s.grant(UnlockKind::ClothingSponsor, "cac_shoe_vans_inventory"));
        assert!(s.is_new(UnlockKind::ClothingUnlock, "cac_unlock_bs_group1"));
        s.clear_new(UnlockKind::ClothingUnlock);
        s.clear_new(UnlockKind::ClothingSponsor);
        assert!(!s.is_new(UnlockKind::ClothingUnlock, "cac_unlock_bs_group1"));
        assert!(s.has(UnlockKind::ClothingSponsor, "cac_shoe_vans_inventory", false));
    }

    #[test]
    fn content_unlocks_count_only_when_asked() {
        let mut s = UnlockStore::default();
        s.grant_content_file("CLOTHING_SPONSOR,cac_throwback_pack_inventory\nNOPE,x\n");
        assert_eq!(s.content.len(), 1);
        assert!(!s.has(UnlockKind::ClothingSponsor, "cac_throwback_pack_inventory", false));
        assert!(s.has(UnlockKind::ClothingSponsor, "cac_throwback_pack_inventory", true));
        assert!(!s.is_new(UnlockKind::ClothingSponsor, "cac_throwback_pack_inventory"));
    }

    #[test]
    fn unlock_all_opens_everything_and_nothing_is_new() {
        let mut s = UnlockStore::default();
        s.grant(UnlockKind::Worlds, "dist_downtown");
        s.unlock_all = true;
        assert!(s.has(UnlockKind::ClothingUnlock, "anything", false));
        assert!(!s.is_new(UnlockKind::Worlds, "dist_downtown"));
    }

    #[test]
    fn load_restores_new_bytes_and_skips_duplicates() {
        let mut saved = UnlockStore::fresh_profile();
        saved.grant(UnlockKind::ClothingUnlock, "cac_unlock_bs_group2");
        let mut s = UnlockStore::fresh_profile();
        s.load(&saved.granted);
        assert_eq!(s, saved);
    }

    #[test]
    fn a_full_table_refuses_grants() {
        let mut s = UnlockStore::default();
        for i in 0..CAPACITY as u32 {
            assert!(s.grant_hash(Table::Granted, UnlockKind::Challenges, i));
        }
        assert!(!s.grant_hash(Table::Granted, UnlockKind::Challenges, u32::MAX));
    }
}
