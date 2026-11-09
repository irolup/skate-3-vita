//! What a created skater actually draws: retail's composition of the worn slots (8253F648, TU3, research of
//! 3 Nov 2026, docs/customization-survey.md). A record's worn slots are not its draw list: the rules below turn them
//! into the list the character is built from, which is what a saved character's composed block holds (the disc's 18
//! recipes are this module's test).
//!
//! The catalog: one [`Record`] per (model, material) of a slot, as retail's table of 0xFC-byte rows, sorted by id,
//! highest first (82DDB710 searches it), so "the first match" below is retail's. Text fields are indices in a shared
//! string table, an empty field [`EMPTY`], as retail's (82DDD7D8 returns -1 for a text it does not hold).
use crate::skater_record::{Gender, Item, SkaterRecord, Slot, SLOTS};
use std::collections::BTreeMap;

/// An empty text field, or a text the catalog does not hold.
pub const EMPTY: u16 = 0xFFFF;

/// The "remove" tile's thumbnail and a missing one (825FF7D0).
pub const THUMB_REMOVE: &str = "fe\\cas\\general\\remove";
pub const THUMB_NONE: &str = "no_image";

/// The catalog fields the rules read (retail's row offset in the comment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    IsColourizable,              // +0x24
    IsDefault,                   // +0x28
    IsDefaultWearStyle,          // +0x2C
    IsLogoable,                  // +0x34
    IsNecklaceRemoved,           // +0x38
    IsSkinned,                   // +0x3C
    IsUsedInGame,                // +0x44
    AlternateMaterialName,       // +0x48
    ArmModelRequired,            // +0x4C
    ArmModelType,                // +0x50
    DeckColour,                  // +0x5C
    DlcID,                       // +0x64
    EyebrowStyle,                // +0x6C
    EyeColour,                   // +0x70
    FacialHairStyle,             // +0x78
    Gender,                      // +0x7C
    HairlineShadowColour,        // +0x80
    HairlineShadowStyle,         // +0x84
    HairModelType,               // +0x88
    HairStyle,                   // +0x8C
    InnerCutType,                // +0x98
    InnerCutTypeRequired,        // +0x9C
    LegModelRequired,            // +0xA0
    LegModelType,                // +0xA4
    ManufacturerID,              // +0xAC
    MaterialVariantName,         // +0xB0
    ModelName,                   // +0xB4
    OuterTorsoMaterial,          // +0xB8
    OuterTorsoModel,             // +0xBC
    RequiresHairlineShadowStyle, // +0xC0
    RequiresHairModelType,       // +0xC4
    RequiresRemovalOfComponents, // +0xC8
    RequiresSockStyle,           // +0xCC
    SkinNormal,                  // +0xD0
    SkinTone,                    // +0xD4
    SockStyle,                   // +0xD8
    SponsorUnlock,               // +0xDC
    StyleSubcategory,            // +0xE0
    SupportsWristItem,           // +0xE4
    TopType,                     // +0xEC
    Unlock,                      // +0xF0
    // Not in retail's 0xFC-byte row layout names above: read from the catalog's own flags for the tattoo pages.
    StampBorderConstraint,       // a stamp's bounds in its part's UV (a Misc row)
    StampUVConstraintQ1,         // +0xF4..: where a stamp may go, by quarter of the part (w4)
    StampUVConstraintQ2,         // w5
    StampUVConstraintQ3,         // w6
    StampUVConstraintQ4,         // w7
    TattooCategory,              // +0xE8
    IsTattooViewingChoice,       // the pants worn while the lower body tattoos are placed
}

impl Field {
    pub const ALL: [Field; 48] = [
        Self::IsColourizable,
        Self::IsDefault,
        Self::IsDefaultWearStyle,
        Self::IsLogoable,
        Self::IsNecklaceRemoved,
        Self::IsSkinned,
        Self::IsUsedInGame,
        Self::AlternateMaterialName,
        Self::ArmModelRequired,
        Self::ArmModelType,
        Self::DeckColour,
        Self::DlcID,
        Self::EyebrowStyle,
        Self::EyeColour,
        Self::FacialHairStyle,
        Self::Gender,
        Self::HairlineShadowColour,
        Self::HairlineShadowStyle,
        Self::HairModelType,
        Self::HairStyle,
        Self::InnerCutType,
        Self::InnerCutTypeRequired,
        Self::LegModelRequired,
        Self::LegModelType,
        Self::ManufacturerID,
        Self::MaterialVariantName,
        Self::ModelName,
        Self::OuterTorsoMaterial,
        Self::OuterTorsoModel,
        Self::RequiresHairlineShadowStyle,
        Self::RequiresHairModelType,
        Self::RequiresRemovalOfComponents,
        Self::RequiresSockStyle,
        Self::SkinNormal,
        Self::SkinTone,
        Self::SockStyle,
        Self::SponsorUnlock,
        Self::StyleSubcategory,
        Self::SupportsWristItem,
        Self::TopType,
        Self::Unlock,
        Self::StampBorderConstraint,
        Self::StampUVConstraintQ1,
        Self::StampUVConstraintQ2,
        Self::StampUVConstraintQ3,
        Self::StampUVConstraintQ4,
        Self::TattooCategory,
        Self::IsTattooViewingChoice,
    ];

    /// The name in the catalog's `cas.` flags (the two names come from the model and its material variant).
    pub fn name(self) -> &'static str {
        match self {
            Self::IsColourizable => "IsColourizable",
            Self::IsDefault => "IsDefault",
            Self::IsDefaultWearStyle => "IsDefaultWearStyle",
            Self::IsLogoable => "IsLogoable",
            Self::IsNecklaceRemoved => "IsNecklaceRemoved",
            Self::IsSkinned => "IsSkinned",
            Self::IsUsedInGame => "IsUsedInGame",
            Self::AlternateMaterialName => "AlternateMaterialName",
            Self::ArmModelRequired => "ArmModelRequired",
            Self::ArmModelType => "ArmModelType",
            Self::DeckColour => "DeckColour",
            Self::DlcID => "DlcID",
            Self::EyebrowStyle => "EyebrowStyle",
            Self::EyeColour => "EyeColour",
            Self::FacialHairStyle => "FacialHairStyle",
            Self::Gender => "Gender",
            Self::HairlineShadowColour => "HairlineShadowColour",
            Self::HairlineShadowStyle => "HairlineShadowStyle",
            Self::HairModelType => "HairModelType",
            Self::HairStyle => "HairStyle",
            Self::InnerCutType => "InnerCutType",
            Self::InnerCutTypeRequired => "InnerCutTypeRequired",
            Self::LegModelRequired => "LegModelRequired",
            Self::LegModelType => "LegModelType",
            Self::ManufacturerID => "ManufacturerID",
            Self::MaterialVariantName => "MaterialVariantName",
            Self::ModelName => "ModelName",
            Self::OuterTorsoMaterial => "OuterTorsoMaterial",
            Self::OuterTorsoModel => "OuterTorsoModel",
            Self::RequiresHairlineShadowStyle => "RequiresHairlineShadowStyle",
            Self::RequiresHairModelType => "RequiresHairModelType",
            Self::RequiresRemovalOfComponents => "RequiresRemovalOfComponents",
            Self::RequiresSockStyle => "RequiresSockStyle",
            Self::SkinNormal => "SkinNormal",
            Self::SkinTone => "SkinTone",
            Self::SockStyle => "SockStyle",
            Self::SponsorUnlock => "SponsorUnlock",
            Self::StyleSubcategory => "StyleSubcategory",
            Self::SupportsWristItem => "SupportsWristItem",
            Self::TopType => "TopType",
            Self::Unlock => "Unlock",
            Self::StampBorderConstraint => "StampBorderConstraint",
            Self::StampUVConstraintQ1 => "StampUVConstraintQ1",
            Self::StampUVConstraintQ2 => "StampUVConstraintQ2",
            Self::StampUVConstraintQ3 => "StampUVConstraintQ3",
            Self::StampUVConstraintQ4 => "StampUVConstraintQ4",
            Self::TattooCategory => "TattooCategory",
            Self::IsTattooViewingChoice => "IsTattooViewingChoice",
        }
    }
}


/// One colour of a zone (`cac_colour`): retail keeps three vectors, the second is the swatch shown on the row (82600620).
#[derive(Clone, Debug, PartialEq)]
pub struct Colour {
    pub key: u64,
    pub primary: [f32; 3],
    pub swatch: [f32; 3],
    pub secondary: [f32; 3],
    pub tag: String,
}

/// A colour zone (`cac_colour_zone`), its colours in their `order` (FUN_8254A078).
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub key: u64,
    /// The colourised slot (the zone's `COLOURIZED_SLOT`), the id 82541310 looks a zone up by.
    pub slot: u32,
    pub colours: Vec<Colour>,
}

/// A morph slider's limits (`cac_morph_params`, 8253CA60; male and female agree on the disc). `index` is the row's +8,
/// the record's weight it moves. Its name's own target is at +4: they differ on nose height and length, as in retail.
#[derive(Clone, Debug, PartialEq)]
pub struct MorphParam {
    pub key: String,
    pub index: usize,
    pub min: f32,
    pub max: f32,
}

/// A face preset (`cac_presets`, 82604208).
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub key: String,
    pub male: bool,
    pub number: u32,
    pub label: String,
    /// By morph row, a value 0 to 1 for its slider.
    pub morphs: Vec<(String, f32)>,
    /// (zone key, colour key).
    pub colours: Vec<(u64, u64)>,
}

/// The preview camera of a row (`cac_camera`): where the eye and what it looks at, with the skater standing at
/// [`Camera::STAGE`]; the field of view in degrees.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub key: String,
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov: f32,
}

impl Camera {
    /// The set's floor: the cameras and the skater's spot (`cac_body`) are offsets from it. Retail adds a stage
    /// position held at run time (`0x830B7D10`); the set's model (`DIST_CreateSakter`) has its floor at y 200. [data]
    pub const STAGE: [f32; 3] = [0.0, 200.0, 0.0];
}

/// Where the skater stands on the screen's set (`cac_body`, 82532800): its spot and heading (degrees, about up), the
/// rate the triggers turn it (8253B618: radians a tick per unit of trigger) and its animation.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub key: String,
    pub position: [f32; 3],
    pub heading: f32,
    pub turn: f32,
    pub anim: String,
}

/// The vault rows the customisation reads (`cas/vault.json`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vault {
    pub zones: Vec<Zone>,
    pub morphs: Vec<MorphParam>,
    pub presets: Vec<Preset>,
    pub cameras: Vec<Camera>,
    pub bodies: Vec<Body>,
    /// A posture's stand (`cac_posture`): by key, its animation.
    pub postures: Vec<(String, String)>,
    /// `cac_hidden_unlocks` `default` (vault class 0x2F68FC82AD102432, read by 825F35A8): the clothing unlocks (field
    /// 0x398276A6376495EE, kind 5) and sponsor inventories (field 0x055CE908E1279D2D, kind 6) whose items the shop
    /// hides until they are granted. [data]
    pub hidden_clothing: Vec<String>,
    pub hidden_sponsors: Vec<String>,
    /// `cac_tattoo_category_order` `default` (field 0xFFDEC982A33E9230) in order: the pages of the tattoo categories.
    pub tattoo_categories: Vec<TattooCategory>,
    /// `cac_stamp_zone`: each zone's key (`tattooupper`, `tattoolower`...) and its `cac_camera`.
    pub stamp_zones: Vec<(String, String)>,
}

/// A tattoo category (`cac_tattoo_categories`): the `TattooCategory` text its stamps carry, and whether the stamps
/// without one are listed here too (the row's field 0x4E26F3294AAED961; only the first category has it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TattooCategory {
    pub text: String,
    pub untagged: bool,
}

impl Vault {
    pub fn zone(&self, slot: u32) -> Option<&Zone> {
        self.zones.iter().find(|z| z.slot == slot)
    }

    pub fn morph(&self, key: &str) -> Option<&MorphParam> {
        self.morphs.iter().find(|m| m.key == key)
    }

    pub fn camera(&self, key: &str) -> Option<&Camera> {
        self.cameras.iter().find(|c| c.key == key)
    }

    pub fn body(&self, key: &str) -> Option<&Body> {
        self.bodies.iter().find(|b| b.key == key)
    }

    /// A stamp zone's camera key (8253E018).
    pub fn stamp_camera(&self, zone: &str) -> Option<&str> {
        self.stamp_zones.iter().find(|(k, _)| k == zone).map(|(_, c)| c.as_str())
    }

    /// The stand of a record's posture (82539688: 1 stiff, 2 slouch, 3 buff, else normal).
    pub fn stand(&self, posture: u32) -> Option<&str> {
        let key = match posture {
            1 => "posture_stiff",
            2 => "posture_slouch",
            3 => "posture_buff",
            _ => "posture_normal",
        };
        self.postures.iter().find(|(k, _)| k == key).map(|(_, a)| a.as_str())
    }
}

const FIELDS: usize = Field::ALL.len();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub item: Item,
    fields: [u16; FIELDS],
}

impl Record {
    fn get(&self, field: Field) -> u16 {
        self.fields[field as usize]
    }

    /// A true flag: any text at all (retail reads a byte, 0 and -1 false).
    fn flag(&self, field: Field) -> bool {
        self.get(field) != EMPTY
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    strings: Vec<String>,
    index: BTreeMap<String, u16>,
    slots: Vec<Vec<Record>>,
    /// A colour layer's vault key to its tag (`cac_colour` rows: `skin_light_2` is `light`).
    tags: BTreeMap<u64, u16>,
}

impl Catalog {
    /// `rows`: per row its slot, item and fields by name (absent or empty: [`EMPTY`]); `tags`: colour key to tag.
    pub fn build(rows: &[(Slot, Item, BTreeMap<String, String>)], tags: &[(u64, String)]) -> Self {
        let mut out = Self { slots: vec![Vec::new(); SLOTS], ..Self::default() };
        for (slot, item, fields) in rows {
            let mut record = Record { item: *item, fields: [EMPTY; FIELDS] };
            for f in Field::ALL {
                if let Some(text) = fields.get(f.name()).filter(|t| !t.is_empty()) {
                    record.fields[f as usize] = out.intern(text);
                }
            }
            out.slots[*slot as usize].push(record);
        }
        for records in &mut out.slots {
            records.sort_by_key(|r| std::cmp::Reverse((r.item.model, r.item.material)));
        }
        for (key, tag) in tags {
            let tag = if tag.is_empty() { EMPTY } else { out.intern(tag) };
            out.tags.insert(*key, tag);
        }
        out
    }

    fn intern(&mut self, text: &str) -> u16 {
        if let Some(&i) = self.index.get(text) {
            return i;
        }
        let i = self.strings.len() as u16;
        self.strings.push(text.to_owned());
        self.index.insert(text.to_owned(), i);
        i
    }

    /// 82DDD7D8.
    fn text(&self, text: &str) -> u16 {
        self.index.get(text).copied().unwrap_or(EMPTY)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        let n = |b: &mut Vec<u8>, v: usize| b.extend_from_slice(&(v as u32).to_le_bytes());
        n(&mut b, FIELDS);
        n(&mut b, self.strings.len());
        for s in &self.strings {
            n(&mut b, s.len());
            b.extend_from_slice(s.as_bytes());
        }
        for records in &self.slots {
            n(&mut b, records.len());
            for r in records {
                b.extend_from_slice(&r.item.model.to_le_bytes());
                b.extend_from_slice(&r.item.material.to_le_bytes());
                r.fields.iter().for_each(|f| b.extend_from_slice(&f.to_le_bytes()));
            }
        }
        n(&mut b, self.tags.len());
        for (key, tag) in &self.tags {
            b.extend_from_slice(&key.to_le_bytes());
            b.extend_from_slice(&tag.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut at = 0;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let end = at + n;
            let s = bytes.get(at..end).ok_or("the CAC catalog ends early")?;
            at = end;
            Ok(s)
        };
        fn u32_of(s: &[u8]) -> usize {
            u32::from_le_bytes(s.try_into().unwrap()) as usize
        }
        fn u64_of(s: &[u8]) -> u64 {
            u64::from_le_bytes(s.try_into().unwrap())
        }
        if u32_of(take(4)?) != FIELDS {
            return Err("the CAC catalog has other fields".into());
        }
        let mut out = Self::default();
        for i in 0..u32_of(take(4)?) {
            let len = u32_of(take(4)?);
            let s = String::from_utf8(take(len)?.to_vec()).map_err(|e| e.to_string())?;
            out.index.insert(s.clone(), i as u16);
            out.strings.push(s);
        }
        for _ in 0..SLOTS {
            let mut records = Vec::new();
            for _ in 0..u32_of(take(4)?) {
                let item = Item { model: u64_of(take(8)?), material: u64_of(take(8)?) };
                let mut fields = [EMPTY; FIELDS];
                for f in &mut fields {
                    *f = u16::from_le_bytes(take(2)?.try_into().unwrap());
                }
                records.push(Record { item, fields });
            }
            out.slots.push(records);
        }
        for _ in 0..u32_of(take(4)?) {
            let key = u64_of(take(8)?);
            out.tags.insert(key, u16::from_le_bytes(take(2)?.try_into().unwrap()));
        }
        Ok(out)
    }

    pub fn records(&self, slot: Slot) -> &[Record] {
        &self.slots[slot as usize]
    }

    /// 82DDB710: a slot's row of an item.
    pub fn get(&self, slot: Slot, item: Item) -> Option<&Record> {
        if item.is_none() {
            return None;
        }
        let key = std::cmp::Reverse((item.model, item.material));
        let rows = self.records(slot);
        rows.binary_search_by_key(&key, |r| std::cmp::Reverse((r.item.model, r.item.material))).ok().map(|i| &rows[i])
    }

    /// The rows a page offers (82603480): the slot's rows of this gender (82DDC438: a row of the other one is out; a
    /// row of neither stays), without the worn-in materials (825368A8) and passing `keep`.
    fn offered(&self, slot: Slot, gender: Gender, keep: impl Fn(&Record) -> bool) -> Vec<Item> {
        let other = self.text(if gender == Gender::Female { "male" } else { "female" });
        self.records(slot).iter().filter(|r| r.get(Field::Gender) != other && !r.flag(Field::IsUsedInGame) && keep(r)).map(|r| r.item).collect()
    }

    /// Hair page (825F3CD0): the hair of the type `full`.
    pub fn hair_rows(&self, gender: Gender) -> Vec<Item> {
        let full = self.text("full");
        self.offered(Slot::Hair, gender, |r| r.get(Field::HairModelType) == full)
    }

    /// The row a page shows for a worn hair, which may be its under-a-hat cut (82600AF8 menu 0x14 with 82DDC250).
    pub fn full_hair_of(&self, worn: Item) -> Option<Item> {
        let row = self.get(Slot::Hair, worn)?;
        self.hair_of(row.get(Field::HairStyle), row.get(Field::Gender), self.text("full")).map(|r| r.item)
    }

    /// Facial Hair page (825F3F38): the heads like the worn one in all but the facial hair.
    pub fn facial_hair_rows(&self, head: Item, gender: Gender) -> Vec<Item> {
        let Some(worn) = self.get(Slot::Rostral, head) else { return Vec::new() };
        let same = [Field::Gender, Field::SkinTone, Field::HairlineShadowColour, Field::HairlineShadowStyle, Field::EyebrowStyle];
        self.offered(Slot::Rostral, gender, |r| same.iter().all(|&f| r.get(f) == worn.get(f)))
    }

    /// Brow page (825F3DA8, sorted by 82630DF8 on the shown style): the heads like the worn one in all but the brow.
    /// A male head has no `style7` (the page's +8 and +0xA, 82600AF8 menu 0x18).
    pub fn brow_rows(&self, head: Item, gender: Gender) -> Vec<Item> {
        let Some(worn) = self.get(Slot::Rostral, head) else { return Vec::new() };
        let same = [Field::Gender, Field::SkinTone, Field::HairlineShadowColour, Field::HairlineShadowStyle, Field::FacialHairStyle];
        let style7 = self.text("style7");
        let male = self.text("male") == worn.get(Field::Gender);
        let mut rows = self.offered(Slot::Rostral, gender, |r| same.iter().all(|&f| r.get(f) == worn.get(f)) && !(male && r.get(Field::EyebrowStyle) == style7));
        rows.sort_by_key(|&item| self.row_text(Slot::Rostral, item, Field::EyebrowStyle).to_ascii_lowercase());
        rows
    }

    /// A field's text ("" when empty).
    pub fn text_of(&self, r: &Record, field: Field) -> &str {
        self.strings.get(usize::from(r.get(field))).map_or("", String::as_str)
    }

    /// A true flag of a row (retail reads a byte: empty and -1 are false).
    pub fn flag_of(&self, r: &Record, field: Field) -> bool {
        r.flag(field)
    }

    /// Does a row's field hold this text (82DDD7D8 then a compare: a text the catalog lacks matches the empty field)?
    pub fn is(&self, r: &Record, field: Field, text: &str) -> bool {
        r.get(field) == self.text(text)
    }

    /// A row's style key (82DDD020): its `StyleSubcategory`, a hat's `MaterialVariantName` when it has none.
    fn style_key(slot: Slot, r: &Record) -> u16 {
        match r.get(Field::StyleSubcategory) {
            EMPTY if slot == Slot::Hat => r.get(Field::MaterialVariantName),
            key => key,
        }
    }

    /// Slappy's rows of a page (82603480 with 825368A8, 82539188 and 826035F0): the slot's rows in catalog order that
    /// are not DLC (no DLC is installed here: retail keeps a DLC row whose pack is installed, 825806C8, 82543300), not
    /// of the other gender, not the worn-in copy, and pass `keep` (the page's class and the unlock filters); one row per
    /// style ([`Catalog::one_per_style`]), in retail's brand order ([`Catalog::brand_order`]).
    pub fn shop_rows(&self, slot: Slot, gender: Gender, keep: impl Fn(&Record) -> bool) -> Vec<Item> {
        let other = self.text(if gender == Gender::Female { "male" } else { "female" });
        let rows: Vec<&Record> = self
            .records(slot)
            .iter()
            .filter(|r| r.get(Field::DlcID) == EMPTY && r.get(Field::Gender) != other && keep(r) && !r.flag(Field::IsUsedInGame))
            .collect();
        self.brand_order(self.one_per_style(slot, rows))
    }

    /// 82539188, on decks, pants, hats and tops: one row per style key, the style's `IsDefaultWearStyle` row where it
    /// stands, else the style's first row moved to the end. Rows without a key stay.
    fn one_per_style<'a>(&self, slot: Slot, rows: Vec<&'a Record>) -> Vec<&'a Record> {
        if !matches!(slot, Slot::SkateBoard | Slot::Pants | Slot::Hat | Slot::OuterTorso) {
            return rows;
        }
        let (mut seen, mut moved, mut out) = (Vec::new(), Vec::<&Record>::new(), Vec::new());
        for r in rows {
            let key = Self::style_key(slot, r);
            if key == EMPTY {
                out.push(r);
            } else if !seen.contains(&key) {
                seen.push(key);
                if r.flag(Field::IsDefaultWearStyle) {
                    out.push(r);
                } else {
                    moved.push(r);
                }
            } else if r.flag(Field::IsDefaultWearStyle) {
                // A default after the style's first: it replaces a first that was moved; after a default, it goes.
                if let Some(at) = moved.iter().position(|m| Self::style_key(slot, m) == key) {
                    moved.remove(at);
                    out.push(r);
                }
            }
        }
        out.extend(moved);
        out
    }

    /// 826035F0: by brand name (`no_brand` for none) with the executable's sort ([`crate::eastl_sort`], which is not
    /// stable), then the colourisable or logoable rows taken out and put first in reverse, the brand `skate` and then
    /// the brand `no_brand` taken out and put last.
    fn brand_order(&self, rows: Vec<&Record>) -> Vec<Item> {
        let name = |r: &Record| match r.get(Field::ManufacturerID) {
            EMPTY => "no_brand",
            t => self.strings.get(usize::from(t)).map_or("no_brand", String::as_str),
        };
        let mut pairs: Vec<(&str, &Record)> = rows.into_iter().map(|r| (name(r), r)).collect();
        crate::eastl_sort::sort(&mut pairs, &|a, b| a.0.as_bytes() < b.0.as_bytes());
        let take = |pairs: &mut Vec<(&str, &Record)>, test: &dyn Fn(&Record) -> bool| -> Vec<Item> {
            let mut out = Vec::new();
            pairs.retain(|(_, r)| {
                let hit = test(r);
                if hit {
                    out.push(r.item);
                }
                !hit
            });
            out
        };
        let custom = take(&mut pairs, &|r| r.flag(Field::IsColourizable) || r.flag(Field::IsLogoable));
        let (skate, none) = (self.text("skate"), self.text("no_brand"));
        let skate_rows = take(&mut pairs, &|r| r.get(Field::ManufacturerID) == skate);
        let none_rows = take(&mut pairs, &|r| r.get(Field::ManufacturerID) == none);
        custom.into_iter().rev().chain(pairs.into_iter().map(|(_, r)| r.item)).chain(skate_rows).chain(none_rows).collect()
    }

    /// The variants of a row (82538D38 with the filter 822FE954 through 825368A8): the row, then the slot's other rows of
    /// its gender and style key (a hat without a `StyleSubcategory` compares its `MaterialVariantName` with the
    /// others' `StyleSubcategory` when they have one), not DLC, not the worn-in copy. A row without a key is alone.
    pub fn variants(&self, slot: Slot, item: Item) -> Vec<Item> {
        let Some(row) = self.get(slot, item) else { return Vec::new() };
        let mut out = vec![item];
        let key = Self::style_key(slot, row);
        if key == EMPTY {
            return out;
        }
        let hat = slot == Slot::Hat;
        for r in self.records(slot) {
            if r.item == item || r.get(Field::Gender) != row.get(Field::Gender) || r.get(Field::DlcID) != EMPTY || r.flag(Field::IsUsedInGame) {
                continue;
            }
            let theirs = if hat && r.get(Field::StyleSubcategory) == EMPTY { r.get(Field::MaterialVariantName) } else { r.get(Field::StyleSubcategory) };
            if theirs == key {
                out.push(r.item);
            }
        }
        out
    }

    /// The text ids that may name a row, in retail's order (82605528): `ID_CAC_PART_<model>_<variant>` of each row of
    /// its gender and style key (82DDB908, the row alone when it has no key), upper case; the first the string table
    /// holds is the name, else the last entry, `ID_CAC_COMPONENT_TYPE_<slot>`.
    pub fn part_names(&self, slot: Slot, item: Item) -> Vec<String> {
        let mut out: Vec<String> = self
            .same_style(slot, item)
            .into_iter()
            .filter_map(|i| self.get(slot, i))
            .map(|r| format!("ID_CAC_PART_{}_{}", self.text_of(r, Field::ModelName), self.text_of(r, Field::MaterialVariantName)).to_ascii_uppercase())
            .collect();
        out.push(format!("ID_CAC_COMPONENT_TYPE_{}", slot.name()).to_ascii_uppercase());
        out
    }

    /// 82DDB908: the slot's rows of a row's gender and style key, in catalog order (the row among them), or the row
    /// alone when it has no key.
    pub fn same_style(&self, slot: Slot, item: Item) -> Vec<Item> {
        let Some(row) = self.get(slot, item) else { return Vec::new() };
        let key = Self::style_key(slot, row);
        if key == EMPTY {
            return vec![item];
        }
        self.records(slot).iter().filter(|r| r.get(Field::Gender) == row.get(Field::Gender) && Self::style_key(slot, r) == key).map(|r| r.item).collect()
    }

    /// Does the catalog hold this text (82DDD7D8 does not return -1)?
    pub fn holds(&self, text: &str) -> bool {
        self.index.contains_key(text)
    }

    /// The variants of a row in Wear Styles' order (82538D38, then 82603CB0: by `ModelName` with the executable's
    /// sort, `no_modelname` for none).
    pub fn variants_by_model(&self, slot: Slot, item: Item) -> Vec<Item> {
        let mut pairs: Vec<(&str, Item)> = self
            .variants(slot, item)
            .into_iter()
            .map(|i| (self.get(slot, i).map(|r| self.text_of(r, Field::ModelName)).filter(|n| !n.is_empty()).unwrap_or("no_modelname"), i))
            .collect();
        crate::eastl_sort::sort(&mut pairs, &|a, b| a.0.as_bytes() < b.0.as_bytes());
        pairs.into_iter().map(|(_, i)| i).collect()
    }

    /// The colour zone a slot's items take (825416A8's table, the zone's colourised slot): hat 0, hair 1, arm, leg
    /// and head 2, eyes 6, outer torso 9, socks 10, deck 11, pants 12. [code]
    pub fn colour_zone(slot: Slot) -> Option<u32> {
        Some(match slot {
            Slot::Hat => 0,
            Slot::Hair => 1,
            Slot::Arm | Slot::Leg | Slot::Rostral => 2,
            Slot::Organ => 6,
            Slot::OuterTorso => 9,
            Slot::Sock => 10,
            Slot::SkateBoard => 11,
            Slot::Pants => 12,
            _ => return None,
        })
    }

    /// The deck a deck colour wears (82DDC160): the first deck, in catalog order, that is not the worn-in copy, is
    /// logoable and has that `DeckColour`.
    pub fn deck_of_colour(&self, tag: &str) -> Option<Item> {
        let tag = self.text(tag);
        self.records(Slot::SkateBoard).iter().find(|r| !r.flag(Field::IsUsedInGame) && r.flag(Field::IsLogoable) && r.get(Field::DeckColour) == tag).map(|r| r.item)
    }

    /// A slot's default item for a gender (82DDBBB8's first pick).
    pub fn default_item(&self, slot: Slot, gender: Gender) -> Option<Item> {
        let g = self.text(if gender == Gender::Male { "male" } else { "female" });
        self.default_of(slot, g).map(|r| r.item)
    }

    /// A row's thumbnail (825FF7D0 with 82604958): `fe\cas\` + a gender folder (shoes: the skater's; else the row's
    /// unless it is unisex) + the slot's name + `\<ModelName>_<MaterialVariantName>`, lower case.
    pub fn thumbnail(&self, slot: Slot, item: Item, skater: Gender) -> String {
        let Some(row) = self.get(slot, item) else { return THUMB_NONE.to_owned() };
        let folder = if slot == Slot::Feet {
            if skater == Gender::Male { "male\\" } else { "female\\" }
        } else {
            match self.text_of(row, Field::Gender) {
                "male" => "male\\",
                "female" => "female\\",
                _ => "",
            }
        };
        format!("fe\\cas\\{folder}{}\\{}_{}", slot.name(), self.text_of(row, Field::ModelName), self.text_of(row, Field::MaterialVariantName)).to_ascii_lowercase()
    }

    /// The top shown when the shirt comes off (825377E0 on the outer torso): the first skinned top of the arms' gender,
    /// in the arms' skin tone and shape (82DDBE88). Without an arm row retail reads the default top's row instead (its
    /// default list, +0x5DF0); here the gender's first default top (82DDBBB8's pick). None: retail wears the default.
    pub fn bare_top(&self, record: &SkaterRecord) -> Option<Item> {
        let gender = self.text(if record.gender == Gender::Male { "male" } else { "female" });
        let arm = self.get(Slot::Arm, record.worn[Slot::Arm as usize]).or_else(|| self.default_of(Slot::OuterTorso, gender))?;
        let first = self.records(Slot::OuterTorso).iter().find(|r| r.flag(Field::IsSkinned) && r.get(Field::Gender) == arm.get(Field::Gender))?;
        Some(self.skinned(first, arm.get(Field::SkinTone), arm.get(Field::SkinNormal)).item)
    }

    /// The slots a page can leave empty (82541008, its table indexed by slot - 2; the outer torso is added by the
    /// page, 82600AF8): hair, torso, accessory (rings), organ, misc, beard, underwear, socks, hat, glasses, wrist,
    /// inner torso, piercings and jewellery. [code]
    pub fn removable(slot: Slot) -> bool {
        const TABLE: [bool; 24] = [
            true, false, true, false, true, false, false, true, true, false, false, false, true, false, true, true, true, false, true, true, true, false,
            true, true,
        ];
        (slot as usize).checked_sub(2).and_then(|i| TABLE.get(i)).copied().unwrap_or(false)
    }

    /// A row's field as its text ("" when it has none).
    pub fn row_text(&self, slot: Slot, item: Item, field: Field) -> &str {
        self.get(slot, item).map(|r| r.get(field)).and_then(|t| self.strings.get(usize::from(t))).map_or("", String::as_str)
    }

    /// The stamps a tattoo category lists (8253E4D8 in its tattoo mode, 8253EC90): the Misc rows in catalog order
    /// whose name holds no `logoEmpty`, does not start with `logo_`, `team_logo_` or `team_brand_logo_`, and whose
    /// `TattooCategory` is the category's text (a row without one only when the category takes untagged rows). [code]
    pub fn tattoo_rows(&self, category: &TattooCategory) -> Vec<Item> {
        self.records(Slot::Misc)
            .iter()
            .filter(|r| {
                let name = self.text_of(r, Field::MaterialVariantName);
                !name.contains("logoEmpty") && !["logo_", "team_logo_", "team_brand_logo_"].iter().any(|p| name.starts_with(p))
            })
            .filter(|r| match r.get(Field::TattooCategory) {
                EMPTY => category.untagged,
                text => self.strings.get(usize::from(text)).is_some_and(|t| *t == category.text),
            })
            .map(|r| r.item)
            .collect()
    }

    /// The logos a Graphics page lists (8253E4D8 with its logo flags, 8253EC90): the brand page (`team` false) takes the
    /// Misc rows not starting `team_logo_`, `tattoo_` or `team_brand_logo_`, the team page the ones not starting `logo_`,
    /// `tattoo_` or `team_brand_logo_`, in catalog order, none containing `logoEmpty`. [code]
    pub fn logo_rows(&self, team: bool) -> Vec<Item> {
        self.records(Slot::Misc)
            .iter()
            .filter(|r| {
                let name = self.text_of(r, Field::MaterialVariantName);
                let kept = if team { !name.starts_with("logo_") } else { !name.starts_with("team_logo_") };
                kept && !name.contains("logoEmpty") && !name.starts_with("tattoo_") && !name.starts_with("team_brand_logo_")
            })
            .map(|r| r.item)
            .collect()
    }

    /// A stamp's name, its `MaterialVariantName` (the item's +0x8C; its picture is `fe/cas/logos/logo_object_<name>`).
    pub fn stamp_name(&self, item: Item) -> &str {
        self.row_text(Slot::Misc, item, Field::MaterialVariantName)
    }

    /// A stamp's description text id (8253DEA0: `ID_CAC_PART_<model>_<variant>`, upper case like the part names).
    pub fn stamp_label(&self, item: Item) -> String {
        let Some(r) = self.get(Slot::Misc, item) else { return String::new() };
        format!("ID_CAC_PART_{}_{}", self.text_of(r, Field::ModelName), self.text_of(r, Field::MaterialVariantName)).to_ascii_uppercase()
    }

    /// A stamp's picture on the thumbnails page (825FF7D0 for a stamp: `fe/cas/logos/logo_object_<name>`).
    pub fn stamp_thumbnail(&self, item: Item) -> String {
        format!("fe/cas/logos/logo_object_{}", self.stamp_name(item)).to_ascii_lowercase()
    }

    /// The pants worn while the lower body is tattooed (825FB8C8: the first of the skater's gender the pants filter
    /// passes; here the rows flagged `IsTattooViewingChoice`). [data]
    pub fn tattoo_pants(&self, gender: Gender) -> Option<Item> {
        let g = self.text(if gender == Gender::Male { "male" } else { "female" });
        self.records(Slot::Pants).iter().find(|r| r.flag(Field::IsTattooViewingChoice) && r.get(Field::Gender) == g).map(|r| r.item)
    }

    /// The stamp of a name (82538998 matches the tattoo row by `MaterialVariantName`).
    pub fn stamp_of_name(&self, name: &str) -> Option<Item> {
        self.records(Slot::Misc).iter().find(|r| self.text_of(r, Field::MaterialVariantName) == name).map(|r| r.item)
    }

    /// Four numbers of a text field `a,b,c,d` (zeros when it has none).
    fn quad(&self, r: &Record, field: Field) -> [f32; 4] {
        let mut out = [0.0; 4];
        for (o, v) in out.iter_mut().zip(self.text_of(r, field).split(',')) {
            *o = v.trim().parse().unwrap_or(0.0);
        }
        out
    }

    /// A stamp's bounds in its picture (82536C40: the tattoo row's, all zero meaning the whole picture, 1 0 0 1).
    pub fn stamp_bounds(&self, item: Item) -> [f32; 4] {
        let b = self.get(Slot::Misc, item).map_or([0.0; 4], |r| self.quad(r, Field::StampBorderConstraint));
        if b == [0.0; 4] { [1.0, 0.0, 0.0, 1.0] } else { b }
    }

    /// Where a stamp may lie on a part at a location 0 to 11 (82536C40): (top, bottom, left, right) in the part's UV,
    /// from the part row's `StampUVConstraintQ1` to `Q4` (locations 0-3, 4-5, 6-8, 9-11), or the quarter's whole
    /// when the row gives none. The row's four numbers are (r0 r1 r2 r3): top 1 - r1, bottom 1 - r0, left r2, right r3.
    pub fn stamp_rect(&self, part: Option<(Slot, Item)>, location: u32) -> [f32; 4] {
        let (field, whole) = match location {
            0..=3 => (Field::StampUVConstraintQ1, [0.5, 0.0, 0.0, 0.5]),
            4..=5 => (Field::StampUVConstraintQ2, [0.5, 0.0, 0.5, 1.0]),
            6..=8 => (Field::StampUVConstraintQ3, [1.0, 0.5, 0.0, 0.5]),
            _ => (Field::StampUVConstraintQ4, [1.0, 0.5, 0.5, 1.0]),
        };
        let q = part.and_then(|(slot, item)| self.get(slot, item)).map_or([0.0; 4], |r| self.quad(r, field));
        if q == [0.0; 4] { whole } else { [1.0 - q[1], 1.0 - q[0], q[2], q[3]] }
    }

    /// The eyes of a colour (82DDC078): the first eyes of the gender with this tag, else the worn ones.
    pub fn eyes_of_colour(&self, worn: Item, tag: &str) -> Item {
        let Some(row) = self.get(Slot::Organ, worn) else { return worn };
        let tag = self.text(tag);
        if row.get(Field::EyeColour) == tag {
            return worn;
        }
        let gender = row.get(Field::Gender);
        self.records(Slot::Organ).iter().find(|r| r.get(Field::Gender) == gender && r.get(Field::EyeColour) == tag).map_or(worn, |r| r.item)
    }

    /// Does the slot's worn row take the skin's colour (`IsSkinned`)?
    pub fn is_skinned(&self, slot: Slot, item: Item) -> bool {
        self.get(slot, item).is_some_and(|r| r.flag(Field::IsSkinned))
    }

    /// A slot's first default row of a gender (82DDBBB8, without its filter pass over several defaults).
    fn default_of(&self, slot: Slot, gender: u16) -> Option<&Record> {
        let unisex = self.text("unisex");
        self.records(slot).iter().find(|r| r.flag(Field::IsDefault) && (r.get(Field::Gender) == gender || r.get(Field::Gender) == unisex))
    }

    /// The hair of a style and type (82DDC250): none for the type `none`; retail's first row when nothing matches.
    fn hair_of(&self, style: u16, gender: u16, kind: u16) -> Option<&Record> {
        if kind == self.text("none") {
            return None;
        }
        let rows = self.records(Slot::Hair);
        rows.iter().find(|r| r.get(Field::HairStyle) == style && r.get(Field::HairModelType) == kind && r.get(Field::Gender) == gender).or_else(|| rows.first())
    }

    /// The arm skin an item asks for (82DDCBA0).
    fn arm_for(&self, wearing: Option<&Record>, tone: u16, shape: u16) -> Option<&Record> {
        let wearing = wearing?;
        let required = wearing.get(Field::ArmModelRequired);
        if required == EMPTY {
            return None;
        }
        self.records(Slot::Arm).iter().find(|r| {
            r.get(Field::Gender) == wearing.get(Field::Gender) && r.get(Field::ArmModelType) == required && r.get(Field::SkinTone) == tone && r.get(Field::SkinNormal) == shape
        })
    }

    /// The leg skin the pants and socks ask for (82DDCCB8).
    fn leg_for(&self, pants: &Record, sock: Option<&Record>, tone: u16) -> Option<&Record> {
        let required = pants.get(Field::LegModelRequired);
        if required == EMPTY {
            return None;
        }
        let none = self.text("none");
        let rows = self.records(Slot::Leg);
        let found = rows.iter().find(|r| {
            r.get(Field::Gender) == pants.get(Field::Gender)
                && r.get(Field::LegModelType) == required
                && match sock {
                    None => r.get(Field::RequiresSockStyle) == none || r.get(Field::RequiresSockStyle) == EMPTY,
                    Some(s) => r.get(Field::RequiresSockStyle) == s.get(Field::SockStyle),
                }
                && r.get(Field::SkinTone) == tone
        });
        if found.is_some() || required != self.text("legs_tight") {
            return found;
        }
        let (unisex, tiny) = (self.text("unisex"), self.text("tiny"));
        rows.iter().find(|r| r.get(Field::Gender) == unisex && r.get(Field::LegModelType) == required && sock.is_some() && r.get(Field::RequiresSockStyle) == tiny)
    }

    /// Clean or worn-in (82DDC350 in the shop, 82DDBAD0 in the game): shoes, decks and wheels come in pairs of materials
    /// on one model, each naming the other as its `AlternateMaterialName`; the worn-in one (`_d`) is `IsUsedInGame`.
    fn wear<'a>(&'a self, slot: Slot, row: &'a Record, in_game: bool) -> &'a Record {
        if row.flag(Field::IsUsedInGame) == in_game {
            return row;
        }
        let alternate = row.get(Field::AlternateMaterialName);
        self.records(slot).iter().find(|r| r.get(Field::ModelName) == row.get(Field::ModelName) && r.get(Field::MaterialVariantName) == alternate).unwrap_or(row)
    }

    /// A skinned top's row of this skin (82DDBE88 for the outer torso): the last row of the model, gender, tone and shape.
    fn skinned<'a>(&'a self, row: &'a Record, tone: u16, shape: u16) -> &'a Record {
        if !row.flag(Field::IsSkinned) {
            return row;
        }
        self.records(Slot::OuterTorso)
            .iter()
            .rfind(|r| r.item.model == row.item.model && r.get(Field::Gender) == row.get(Field::Gender) && r.get(Field::SkinTone) == tone && r.get(Field::SkinNormal) == shape)
            .unwrap_or(row)
    }

    /// The composition (8253F648): the record's worn slots as the list the character draws. `in_game` is its last
    /// argument: the worn-in materials while skating, the clean ones in the shop.
    pub fn compose(&self, record: &SkaterRecord, in_game: bool) -> [Item; SLOTS] {
        let mut list = record.worn;
        let tag = |key: u64| self.tags.get(&key).copied().unwrap_or(EMPTY);
        let hair_colour = tag(record.colour_keys[1]);
        let tone = tag(record.colour_keys[2]);
        let shape = self.text(if record.body_shape.is_empty() { "average" } else { &record.body_shape });
        let row = |list: &[Item; SLOTS], slot: Slot| self.get(slot, list[slot as usize]);
        let set = |list: &mut [Item; SLOTS], slot: Slot, r: Option<&Record>| list[slot as usize] = r.map_or(Item::NONE, |r| r.item);

        // Which hair the hat lets through: a hat's RequiresHairModelType `full` or no hat 2, `none` 0, else 1.
        let hat = row(&list, Slot::Hat);
        let hair_state = match hat {
            None => 2,
            Some(h) if h.get(Field::RequiresHairModelType) == self.text("full") => 2,
            Some(h) if h.get(Field::RequiresHairModelType) == self.text("none") => 0,
            Some(_) => 1,
        };
        // 82540538.
        let hair = row(&list, Slot::Hair);
        let hat_type = self.text("hat");
        let new_hair = match (hair_state, hair) {
            (0, _) | (_, None) => None,
            (1, Some(h)) if h.get(Field::HairModelType) == hat_type => Some(h),
            (1, Some(h)) => {
                let kind = hat.map_or(self.text("none"), |x| x.get(Field::RequiresHairModelType));
                self.hair_of(h.get(Field::HairStyle), h.get(Field::Gender), kind)
            }
            (_, Some(h)) if h.get(Field::HairModelType) != hat_type => Some(h),
            (_, Some(h)) => self.hair_of(h.get(Field::HairStyle), h.get(Field::Gender), self.text("full")),
        };
        set(&mut list, Slot::Hair, new_hair);

        // The head's material follows the hairline, skin, facial hair and brows (825406E0, 82DDCA28): the last match.
        if let Some(head) = row(&list, Slot::Rostral) {
            let hairline = new_hair.map_or(self.text("0"), |h| h.get(Field::RequiresHairlineShadowStyle));
            let found = self.records(Slot::Rostral).iter().rfind(|r| {
                r.get(Field::Gender) == head.get(Field::Gender)
                    && r.get(Field::SkinTone) == tone
                    && r.get(Field::HairlineShadowStyle) == hairline
                    && r.get(Field::HairlineShadowColour) == hair_colour
                    && r.get(Field::FacialHairStyle) == head.get(Field::FacialHairStyle)
                    && r.get(Field::EyebrowStyle) == head.get(Field::EyebrowStyle)
            });
            if let Some(f) = found {
                list[Slot::Rostral as usize] = f.item;
            }
        }

        // The torso (82540828): the top, its inner layer if its cut asks for one, the arm skin they ask for.
        let inner = row(&list, Slot::InnerTorso);
        let outer = match (row(&list, Slot::OuterTorso), inner) {
            (Some(o), _) => Some(o),
            (None, None) => None,
            // 82540AE0: the top an inner layer belongs to, else the default.
            (None, Some(i)) => {
                let material = match i.get(Field::OuterTorsoMaterial) {
                    EMPTY => i.get(Field::MaterialVariantName),
                    m => m,
                };
                self.records(Slot::OuterTorso)
                    .iter()
                    .find(|r| r.get(Field::ModelName) == i.get(Field::OuterTorsoModel) && r.get(Field::MaterialVariantName) == material)
                    .or_else(|| self.default_of(Slot::OuterTorso, i.get(Field::Gender)))
            }
        };
        // 82540BF8.
        let inner = outer.and_then(|o| {
            let required = o.get(Field::InnerCutTypeRequired);
            if required == EMPTY {
                return None;
            }
            let i = inner.or_else(|| self.default_of(Slot::InnerTorso, o.get(Field::Gender)))?;
            if i.get(Field::InnerCutType) == required {
                return Some(i);
            }
            self.records(Slot::InnerTorso).iter().find(|r| r.item.material == i.item.material && r.get(Field::InnerCutType) == required)
        });
        let arm = self.arm_for(inner, tone, shape).or_else(|| self.arm_for(outer, tone, shape));
        set(&mut list, Slot::OuterTorso, outer.map(|o| self.skinned(o, tone, shape)));
        set(&mut list, Slot::InnerTorso, inner);
        if let Some(a) = arm {
            list[Slot::Arm as usize] = a.item;
        } else {
            list[Slot::Arm as usize] = Item::NONE;
        }

        // 82540430: a top that takes the necklace off.
        if [inner, outer].iter().flatten().any(|r| r.flag(Field::IsNecklaceRemoved)) {
            list[Slot::Jewellery as usize] = Item::NONE;
        }
        // A wrist item needs an arm that shows it. Retail compares with `yes`; the catalog only ever writes `no` (two
        // long-sleeve arms), a blank being yes.
        if !list[Slot::WristItem as usize].is_none() {
            if let Some(a) = row(&list, Slot::Arm) {
                if a.get(Field::SupportsWristItem) == self.text("no") {
                    list[Slot::WristItem as usize] = Item::NONE;
                }
            }
        }
        // The leg skin under the pants (82DDCCB8), then the socks it allows (82540D28, 82DDCF10).
        let sock = row(&list, Slot::Sock);
        let leg = row(&list, Slot::Pants).and_then(|p| self.leg_for(p, sock, tone));
        set(&mut list, Slot::Leg, leg);
        let new_sock = match (leg, sock) {
            (Some(l), Some(s)) => {
                let required = l.get(Field::RequiresSockStyle);
                if required == EMPTY {
                    None
                } else if s.get(Field::SockStyle) == required {
                    Some(s)
                } else {
                    self.records(Slot::Sock).iter().find(|r| r.get(Field::SockStyle) == required)
                }
            }
            _ => None,
        };
        if new_sock.is_some_and(|s| s.get(Field::SockStyle) == self.text("tiny")) {
            list[Slot::Leg as usize] = Item::NONE;
        }
        set(&mut list, Slot::Sock, new_sock);
        // Clean or worn-in shoes, deck and wheels (82540E20 for the last two).
        for slot in [Slot::Feet, Slot::SkateBoard, Slot::SkateWheel] {
            if let Some(r) = row(&list, slot) {
                list[slot as usize] = self.wear(slot, r, in_game).item;
            }
        }
        // 8253FD40: items that take others off.
        for slot in Slot::ALL {
            let Some(r) = row(&list, slot) else { continue };
            let removal = r.get(Field::RequiresRemovalOfComponents);
            if removal == EMPTY {
                continue;
            }
            for name in self.strings[usize::from(removal)].split(',') {
                if let Some(s) = Slot::from_name(name) {
                    list[s as usize] = Item::NONE;
                }
            }
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(f: &[(&str, &str)]) -> BTreeMap<String, String> {
        f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
    const TANK: Item = Item { model: 1, material: 10 };
    const JACKET: Item = Item { model: 2, material: 20 };
    const INNER: Item = Item { model: 3, material: 30 };
    const ARM_BARE: Item = Item { model: 4, material: 40 };
    const ARM_WRIST: Item = Item { model: 5, material: 50 };
    const HAIR_FULL: Item = Item { model: 6, material: 60 };
    const HAIR_HAT: Item = Item { model: 7, material: 60 };
    const CAP: Item = Item { model: 8, material: 80 };
    const SHOE_SHOP: Item = Item { model: 9, material: 90 };
    const SHOE_GAME: Item = Item { model: 9, material: 91 };
    const HOOD: Item = Item { model: 11, material: 110 };

    fn catalog() -> Catalog {
        let m = "male";
        let rows = vec![
            (Slot::OuterTorso, TANK, fields(&[("Gender", m), ("ArmModelRequired", "shoulder")])),
            (Slot::OuterTorso, JACKET, fields(&[("Gender", m), ("ArmModelRequired", "wrist"), ("InnerCutTypeRequired", "open")])),
            (Slot::OuterTorso, HOOD, fields(&[("Gender", m), ("ArmModelRequired", "wrist"), ("RequiresRemovalOfComponents", "Hat,Hair")])),
            (Slot::InnerTorso, INNER, fields(&[("Gender", m), ("InnerCutType", "open")])),
            (Slot::Arm, ARM_BARE, fields(&[("Gender", m), ("ArmModelType", "shoulder"), ("SkinTone", "light"), ("SkinNormal", "average")])),
            (Slot::Arm, ARM_WRIST, fields(&[("Gender", m), ("ArmModelType", "wrist"), ("SkinTone", "light"), ("SkinNormal", "average")])),
            (Slot::Hair, HAIR_FULL, fields(&[("Gender", m), ("HairStyle", "ceasar"), ("HairModelType", "full")])),
            (Slot::Hair, HAIR_HAT, fields(&[("Gender", m), ("HairStyle", "ceasar"), ("HairModelType", "hat")])),
            (Slot::Hat, CAP, fields(&[("Gender", m), ("RequiresHairModelType", "hat")])),
            (Slot::Feet, SHOE_SHOP, fields(&[("ModelName", "shoe"), ("MaterialVariantName", "shop"), ("AlternateMaterialName", "game")])),
            (Slot::Feet, SHOE_GAME, fields(&[("ModelName", "shoe"), ("MaterialVariantName", "game"), ("IsUsedInGame", "1"), ("AlternateMaterialName", "shop")])),
        ];
        Catalog::build(&rows, &[(100, "black".into()), (200, "light".into())])
    }

    fn skater(worn: &[(Slot, Item)]) -> SkaterRecord {
        let mut r = SkaterRecord::cleared();
        r.body_shape = "average".into();
        r.colour_keys[1] = 100;
        r.colour_keys[2] = 200;
        for (s, i) in worn {
            r.worn[*s as usize] = *i;
        }
        r
    }

    #[test]
    fn an_inner_torso_shows_only_under_a_top_whose_cut_asks_for_it() {
        let c = catalog();
        let tank = c.compose(&skater(&[(Slot::OuterTorso, TANK), (Slot::InnerTorso, INNER)]), true);
        assert!(tank[Slot::InnerTorso as usize].is_none());
        assert_eq!(tank[Slot::Arm as usize], ARM_BARE);
        let jacket = c.compose(&skater(&[(Slot::OuterTorso, JACKET), (Slot::InnerTorso, INNER)]), true);
        assert_eq!(jacket[Slot::InnerTorso as usize], INNER);
        assert_eq!(jacket[Slot::Arm as usize], ARM_WRIST);
    }

    #[test]
    fn hair_follows_the_hat() {
        let c = catalog();
        let bare = c.compose(&skater(&[(Slot::Hair, HAIR_HAT)]), true);
        assert_eq!(bare[Slot::Hair as usize], HAIR_FULL);
        let capped = c.compose(&skater(&[(Slot::Hair, HAIR_FULL), (Slot::Hat, CAP)]), true);
        assert_eq!(capped[Slot::Hair as usize], HAIR_HAT);
    }

    #[test]
    fn a_hood_takes_the_hat_and_hair_off_and_shoes_use_their_game_material() {
        let c = catalog();
        let worn = skater(&[(Slot::OuterTorso, HOOD), (Slot::Hair, HAIR_FULL), (Slot::Hat, CAP), (Slot::Feet, SHOE_SHOP)]);
        let list = c.compose(&worn, true);
        assert!(list[Slot::Hair as usize].is_none() && list[Slot::Hat as usize].is_none());
        assert_eq!(list[Slot::Feet as usize], SHOE_GAME);
        // The shop shows the clean pair, from either.
        assert_eq!(c.compose(&worn, false)[Slot::Feet as usize], SHOE_SHOP);
        assert_eq!(c.compose(&skater(&[(Slot::Feet, SHOE_GAME)]), false)[Slot::Feet as usize], SHOE_SHOP);
    }

    #[test]
    fn the_catalog_survives_its_bytes() {
        let c = catalog();
        assert_eq!(Catalog::from_bytes(&c.to_bytes()).unwrap(), c);
        assert!(Catalog::from_bytes(&c.to_bytes()[..20]).is_err());
    }
}
