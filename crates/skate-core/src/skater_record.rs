//! One created skater: retail's CAC character (0x5D60 bytes; TU3, research of 2 Nov 2026,
//! docs/customization-survey.md). `CACBackEnd::Data` (save 825199B8, load 82519878, version 8) keeps 5 of them at
//! 0x8309A800; each is written by 82DE0738 and read by 82DE0408. What the CAC edits, the career's team and the Free
//! Play "You" row read.
//!
//! Fields and their retail offsets:
//! - Edit Skater (section version 0xC, 82DDE760): trucks +0x2004 and wheels +0x2008 (floats, Loose / Soft below
//!   0.35, Tight / Hard above 0.75, steps of 0.1: 825FFE08, 825FC7E8), stance +0x2010 (0 goofy, else regular), style
//!   +0x2014 (0-3, shown as Standard, Loose, OG, Aggressive), posture +0x2018 (0-3, shown as Default, Straight,
//!   Hunched, Macho), the four gesture slots +0x201C, the gender
//!   +0x2030 (1 male) and the body shape name +0x2031 (64 bytes, "average"). +0x200C and +0x202C are saved, not named.
//! - Worn items +0x5B20: one (model id, material id) pair per component slot, the catalog's 64-bit asset ids. An empty
//!   slot holds [`Item::NONE`] (0x830CAB18, set at run time; its value read from the disc's recipes).
//! - The closet +0x5CC0: per slot, the items owned but not worn (at most 6,000 in all, padded to 6,000 when saved).
//! - Colour layers (sub-object +0x5CE4 list 0, at most 13): index, two RGB colours, the (model, material) they tint
//!   (the disc's recipes: 1 hair, 2 head, 3 arms, 5 inner torso); their vault keys at +0x5CE8 (all ones: none).
//! - Graphics (list 1, at most 5): a name, three 64-bit values, two bytes; and their adjustments (+0xC of the
//!   sub-object: five words, five times four floats, a byte), the five Graphic Adjust menus.
//! - Morphs (list 2): 19 weights, each its target's weight directly (0 to 0.5), in retail's `target_order` (the
//!   vault's morph rows: fat, thin, then the face modification pages).
//!
//! Not kept here (yet): the 15,000-byte thumbnail +0x2080 (the skater's portrait, uploaded online and shown on the
//! team screens), the 8 KB block +0x0000 (length +0x2000), and the per-gender snapshots (+0x5CE0: switching gender
//! restores the other gender's last look).

/// Retail's component slots, in index order (name table 0x82FCA0A0; the checkout names items through it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    Arm = 0,
    Leg,
    Hair,
    Rostral,
    Torso,
    Feet,
    Accessory,
    Equipment,
    Skeleton,
    Organ,
    Misc,
    SkateBoard,
    SkateTruck,
    SkateWheel,
    Beard,
    Pants,
    Underwear,
    Sock,
    Hat,
    Belt,
    Glasses,
    WristItem,
    InnerTorso,
    OuterTorso,
    Piercings,
    Jewellery,
}

pub const SLOTS: usize = 26;

impl Slot {
    pub const ALL: [Slot; SLOTS] = [
        Self::Arm,
        Self::Leg,
        Self::Hair,
        Self::Rostral,
        Self::Torso,
        Self::Feet,
        Self::Accessory,
        Self::Equipment,
        Self::Skeleton,
        Self::Organ,
        Self::Misc,
        Self::SkateBoard,
        Self::SkateTruck,
        Self::SkateWheel,
        Self::Beard,
        Self::Pants,
        Self::Underwear,
        Self::Sock,
        Self::Hat,
        Self::Belt,
        Self::Glasses,
        Self::WristItem,
        Self::InnerTorso,
        Self::OuterTorso,
        Self::Piercings,
        Self::Jewellery,
    ];

    /// The name in retail's table, as the catalog's `comp n=` uses it.
    pub fn name(self) -> &'static str {
        [
            "Arm", "Leg", "Hair", "Rostral", "Torso", "Feet", "Accessory", "Equipment", "Skeleton", "Organ", "Misc",
            "SkateBoard", "SkateTruck", "SkateWheel", "Beard", "Pants", "Underwear", "Sock", "Hat", "Belt", "Glasses",
            "WristItem", "InnerTorso", "OuterTorso", "Piercings", "Jewellery",
        ][self as usize]
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// A catalog item: the part model and its material variant (64-bit asset ids).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Item {
    pub model: u64,
    pub material: u64,
}

impl Item {
    pub const NONE: Item = Item { model: 0x0167_8C48_6213_9059, material: 0x0167_8C48_6213_9059 };

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gender {
    Female = 0,
    Male = 1,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourLayer {
    pub index: u8,
    pub primary: [f32; 3],
    pub secondary: [f32; 3],
    /// The item this layer tints ([`Item::NONE`]: none now).
    pub part: Item,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graphic {
    pub name: String,
    pub values: [u64; 3],
    pub flags: [u8; 2],
}

/// The five Graphic Adjust records (sub-object +0xC).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphicAdjust {
    pub words: [u32; 5],
    pub values: [[f32; 4]; 5],
    pub flag: u8,
}

/// A cleared record's adjustments (825315E8): no region (-1) and the placement (0, 1, -1, -1) in every slot.
impl Default for GraphicAdjust {
    fn default() -> Self {
        Self { words: [u32::MAX; 5], values: [[0.0, 1.0, -1.0, -1.0]; 5], flag: 0 }
    }
}

pub const COLOUR_LAYERS: usize = 13;
pub const GRAPHICS: usize = 5;
pub const MORPHS: usize = 19;
/// Closet entries in all (82DE0738 stops at 6,000).
pub const CLOSET: usize = 6000;

#[derive(Clone, Debug, PartialEq)]
pub struct SkaterRecord {
    pub gender: Gender,
    pub body_shape: String,
    pub trucks: f32,
    pub wheels: f32,
    /// 0 goofy, else regular.
    pub stance: u32,
    pub style: u32,
    pub posture: u32,
    pub gestures: [u32; 4],
    /// +0x200C and +0x202C: saved, meaning not read yet.
    pub unknown: [u32; 2],
    pub worn: [Item; SLOTS],
    pub closet: [Vec<Item>; SLOTS],
    pub colour_keys: [u64; COLOUR_LAYERS],
    pub colours: Vec<ColourLayer>,
    pub graphics: Vec<Graphic>,
    pub adjust: GraphicAdjust,
    pub morphs: Vec<f32>,
}

impl SkaterRecord {
    /// A cleared record (82DDEBF8 with 82DDE700): no items, trucks and wheels 0, stance 0 (goofy), style 3 (shown as
    /// Aggressive), posture 0 (Default), the first four gestures (824FA730), male. A new skater keeps these style
    /// values: opening the CAC on a fresh slot only resets it (825FAFF0 > 82519538, which adds the 0.7 trucks and
    /// wheels), and the disc's fallback recipes hold the same. The default outfit, colours and morphs come from the
    /// catalog (8253D470) once it is baked.
    pub fn cleared() -> Self {
        Self {
            gender: Gender::Male,
            body_shape: String::new(),
            trucks: 0.0,
            wheels: 0.0,
            stance: 0,
            style: 3,
            posture: 0,
            gestures: [0, 1, 2, 3],
            unknown: [0, 1],
            worn: [Item::NONE; SLOTS],
            closet: std::array::from_fn(|_| Vec::new()),
            colour_keys: [u64::MAX; COLOUR_LAYERS],
            colours: Vec::new(),
            graphics: Vec::new(),
            adjust: GraphicAdjust::default(),
            morphs: vec![0.0; MORPHS],
        }
    }

    /// Does the skater own it: worn in a slot or in that slot's closet (82DDF060 over every slot)?
    pub fn owns(&self, item: Item) -> bool {
        self.worn.contains(&item) || self.closet.iter().any(|c| c.contains(&item))
    }

    /// Take an item out of a slot (82DDF150): the worn one is cleared, else its first copy leaves the closet.
    pub fn take(&mut self, slot: Slot, item: Item) -> bool {
        if item.is_none() {
            return false;
        }
        let s = slot as usize;
        if self.worn[s] == item {
            self.worn[s] = Item::NONE;
            return true;
        }
        match self.closet[s].iter().position(|&i| i == item) {
            Some(at) => {
                self.closet[s].remove(at);
                true
            }
            None => false,
        }
    }

    /// Put an item away (82DDEF68): into the slot's closet unless the skater already owns it.
    pub fn stash(&mut self, slot: Slot, item: Item) {
        if !item.is_none() && !self.owns(item) && self.closet_len() < CLOSET {
            self.closet[slot as usize].push(item);
        }
    }

    /// Wear an item (82DDEEF8): it leaves the closet, and the item worn before is put away.
    pub fn wear(&mut self, slot: Slot, item: Item) {
        let old = self.worn[slot as usize];
        self.take(slot, item);
        self.worn[slot as usize] = item;
        self.stash(slot, old);
    }

    /// Wear a new item (82DDEDB0); with `keep`, a copy also goes in the closet. An empty item changes nothing.
    pub fn wear_new(&mut self, slot: Slot, item: Item, keep: bool) {
        if item.is_none() {
            return;
        }
        if keep {
            self.take(slot, item);
            self.closet[slot as usize].push(item);
        }
        self.worn[slot as usize] = item;
    }

    pub fn closet_len(&self) -> usize {
        self.closet.iter().map(Vec::len).sum()
    }
}

/// A saved character is written in the console's byte order.
struct Retail<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Retail<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or_else(|| format!("record ends at {} of {n} more bytes", self.at))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32s<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
    fn item(&mut self) -> Result<Item, String> {
        Ok(Item { model: self.u64()?, material: self.u64()? })
    }
    fn expect(&mut self, what: &str, version: u32) -> Result<(), String> {
        match self.u32()? {
            v if v == version => Ok(()),
            v => Err(format!("{what} version {v}, expected {version}")),
        }
    }
}

/// One per-gender snapshot in a saved character (82DDAB00: 15,008 thumbnail, 13 x 48 colours, gender, worn, morphs,
/// 5 x 293 graphics, the adjustments, style values).
const SNAPSHOT: usize = 17763;

impl SkaterRecord {
    /// A saved character as the console writes it (82DE0738; read by 82DE0408): the disc's `data\cacrecipes` files.
    /// The composed block, thumbnail and per-gender snapshots are skipped.
    pub fn from_retail(bytes: &[u8]) -> Result<Self, String> {
        let mut r = Retail { bytes, at: 0 };
        let mut out = Self::cleared();
        r.expect("character", 8)?;
        r.u32()?;
        // The section at +0x2000 (82DDE760).
        r.expect("section", 0xC)?;
        let composed = r.u32()? as usize;
        r.take(composed)?;
        out.trucks = r.f32()?;
        out.wheels = r.f32()?;
        out.unknown[0] = r.u32()?;
        out.stance = r.u32()?;
        out.style = r.u32()?;
        out.posture = r.u32()?;
        for g in &mut out.gestures {
            *g = r.u32()?;
        }
        out.gender = if r.u8()? == 0 { Gender::Female } else { Gender::Male };
        out.unknown[1] = r.u32()?;
        let shape = r.take(64)?;
        out.body_shape = String::from_utf8_lossy(&shape[..shape.iter().position(|&b| b == 0).unwrap_or(64)]).into_owned();
        // The thumbnail and its length.
        r.take(15000)?;
        r.u32()?;
        for worn in &mut out.worn {
            *worn = r.item()?;
        }
        let mut owned = 0;
        for closet in &mut out.closet {
            let n = r.u32()? as usize;
            owned += n;
            for _ in 0..n {
                closet.push(r.item()?);
            }
        }
        r.take(CLOSET.checked_sub(owned).ok_or("closet over 6,000 items")? * 16)?;
        for key in &mut out.colour_keys {
            *key = r.u64()?;
        }
        // The per-gender snapshots (82DDB250).
        r.expect("snapshots", 1)?;
        r.u32()?;
        r.u32()?;
        let snapshots = r.u32()? as usize;
        r.take(snapshots.checked_mul(SNAPSHOT).ok_or("too many snapshots")?)?;
        // The sub-object (82C2CF38).
        r.expect("appearance", 6)?;
        r.u8()?;
        r.u32()?;
        for _ in 0..r.u32()? {
            out.colours.push(ColourLayer { index: r.u32()? as u8, primary: r.f32s()?, secondary: r.f32s()?, part: r.item()? });
        }
        r.u32()?;
        for _ in 0..r.u32()? {
            let len = r.u32()? as usize;
            let name = String::from_utf8_lossy(r.take(len)?).into_owned();
            out.graphics.push(Graphic { name, values: [r.u64()?, r.u64()?, r.u64()?], flags: [r.u8()?, r.u8()?] });
        }
        out.morphs.clear();
        for _ in 0..r.u32()? {
            out.morphs.push(r.f32()?);
        }
        for w in &mut out.adjust.words {
            *w = r.u32()?;
        }
        for v in &mut out.adjust.values {
            *v = r.f32s()?;
        }
        out.adjust.flag = r.u8()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEE: Item = Item { model: 0xadd5_a352_7085_978f, material: 0x0000_2387_03e3_8817 };
    const HOODIE: Item = Item { model: 0x2c7f_3811_0017_0160, material: 0x0000_2386_03e3_8817 };

    #[test]
    fn slots_round_trip() {
        for s in Slot::ALL {
            assert_eq!(Slot::from_name(s.name()), Some(s));
        }
        assert_eq!(Slot::Jewellery as usize, SLOTS - 1);
    }

    #[test]
    fn cleared_matches_retail_reset() {
        let r = SkaterRecord::cleared();
        assert_eq!((r.stance, r.style, r.posture, r.gestures), (0, 3, 0, [0, 1, 2, 3]));
        assert!(r.worn.iter().all(|i| i.is_none()));
        assert_eq!(r.morphs.len(), MORPHS);
    }

    #[test]
    fn wearing_puts_the_old_item_in_the_closet() {
        let mut r = SkaterRecord::cleared();
        r.wear(Slot::OuterTorso, TEE);
        assert_eq!(r.closet_len(), 0);
        r.wear(Slot::OuterTorso, HOODIE);
        assert_eq!(r.closet[Slot::OuterTorso as usize], [TEE]);
        // Wearing it again takes it out of the closet and puts the hoodie away.
        r.wear(Slot::OuterTorso, TEE);
        assert_eq!(r.closet[Slot::OuterTorso as usize], [HOODIE]);
        assert!(r.owns(TEE) && r.owns(HOODIE));
    }

    #[test]
    fn taking_off_leaves_the_slot_empty_and_keeps_the_item() {
        let mut r = SkaterRecord::cleared();
        r.wear(Slot::Hat, TEE);
        r.wear(Slot::Hat, Item::NONE);
        assert!(r.worn[Slot::Hat as usize].is_none());
        assert_eq!(r.closet[Slot::Hat as usize], [TEE]);
    }

    #[test]
    fn a_retail_record_reads_field_by_field() {
        let mut b = Vec::new();
        let u32s = |b: &mut Vec<u8>, v: &[u32]| v.iter().for_each(|x| b.extend_from_slice(&x.to_be_bytes()));
        let f32s = |b: &mut Vec<u8>, v: &[f32]| v.iter().for_each(|x| b.extend_from_slice(&x.to_be_bytes()));
        let item = |b: &mut Vec<u8>, i: Item| [i.model, i.material].iter().for_each(|x| b.extend_from_slice(&x.to_be_bytes()));
        u32s(&mut b, &[8, 3, 0xC, 3]);
        b.extend_from_slice(b"abc");
        f32s(&mut b, &[0.7, 0.4]);
        u32s(&mut b, &[0, 1, 2, 3, 9, 8, 7, 6]);
        b.push(0);
        u32s(&mut b, &[1]);
        let mut shape = [0u8; 64];
        shape[..7].copy_from_slice(b"average");
        b.extend_from_slice(&shape);
        b.extend_from_slice(&[0; 15000]);
        u32s(&mut b, &[0]);
        for s in Slot::ALL {
            item(&mut b, if s == Slot::Hat { TEE } else { Item::NONE });
        }
        for s in Slot::ALL {
            u32s(&mut b, &[u32::from(s == Slot::Feet)]);
            if s == Slot::Feet {
                item(&mut b, HOODIE);
            }
        }
        b.extend_from_slice(&vec![0; (CLOSET - 1) * 16]);
        b.extend_from_slice(&[0xFF; COLOUR_LAYERS * 8]);
        u32s(&mut b, &[1, 0, 1, 2]);
        b.extend_from_slice(&[0; 2 * SNAPSHOT]);
        u32s(&mut b, &[6]);
        b.push(0);
        u32s(&mut b, &[13, 1, 2]);
        f32s(&mut b, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        item(&mut b, TEE);
        u32s(&mut b, &[5, 1, 4]);
        b.extend_from_slice(b"logo");
        b.extend_from_slice(&[0; 24]);
        b.extend_from_slice(&[1, 0]);
        u32s(&mut b, &[2]);
        f32s(&mut b, &[0.0, 0.25]);
        u32s(&mut b, &[5, 4, 3, 2, 1]);
        f32s(&mut b, &[1.0; 20]);
        b.push(1);
        // Retail's files carry trailing bytes.
        b.extend_from_slice(&[7; 40]);
        let r = SkaterRecord::from_retail(&b).unwrap();
        assert_eq!((r.gender, r.trucks, r.wheels, r.stance, r.style, r.posture), (Gender::Female, 0.7, 0.4, 1, 2, 3));
        assert_eq!((r.gestures, r.body_shape.as_str()), ([9, 8, 7, 6], "average"));
        assert_eq!((r.worn[Slot::Hat as usize], r.worn[Slot::Arm as usize]), (TEE, Item::NONE));
        assert_eq!(r.closet[Slot::Feet as usize], [HOODIE]);
        assert_eq!((r.colours[0].index, r.colours[0].part, r.colours[0].secondary[2]), (2, TEE, 0.6));
        assert_eq!((r.graphics[0].name.as_str(), r.graphics[0].flags, r.morphs.as_slice()), ("logo", [1, 0], [0.0, 0.25].as_slice()));
        assert_eq!((r.adjust.words[4], r.adjust.flag), (1, 1));
        assert!(SkaterRecord::from_retail(&b[..100]).is_err());
        b[3] = 7;
        assert!(SkaterRecord::from_retail(&b).is_err());
    }

    #[test]
    fn a_new_item_kept_is_worn_and_in_the_closet() {
        let mut r = SkaterRecord::cleared();
        r.wear_new(Slot::Feet, TEE, true);
        assert_eq!(r.worn[Slot::Feet as usize], TEE);
        assert_eq!(r.closet[Slot::Feet as usize], [TEE]);
        // Again while worn: the take clears the worn slot, not the closet copy, so retail keeps a second copy.
        r.wear_new(Slot::Feet, TEE, true);
        assert_eq!(r.closet[Slot::Feet as usize], [TEE, TEE]);
        r.wear_new(Slot::Feet, Item::NONE, true);
        assert_eq!(r.worn[Slot::Feet as usize], TEE);
    }
}
