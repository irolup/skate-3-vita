//! Typed access to converted stock skater XML values, without engine objects.
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Deserialize)]
pub struct Field {
    #[serde(rename = "type")]
    pub type_name: TypeName,
    pub data: Interned,
    /// An array field's elements (`data` holds the count): the converter's `array`, with the
    /// text elements resolved (`text_items`).
    #[serde(default)]
    pub array: Option<FieldArray>,
}

/// skate_vita (memory): a field's type name or data, shared: the stock collections name 193
/// types and 11,094 distinct values over 87,668 fields, each of which held two `String`s of its
/// own. Compares and reads as the string it stands for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Interned(&'static str);

pub type TypeName = Interned;

impl Interned {
    pub fn new(name: &str) -> Self {
        static NAMES: Mutex<Option<std::collections::HashSet<&'static str>>> = Mutex::new(None);
        let mut names = NAMES.lock().unwrap();
        let names = names.get_or_insert_with(Default::default);
        match names.get(name) {
            Some(known) => Self(known),
            None => {
                let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
                names.insert(leaked);
                Self(leaked)
            }
        }
    }

    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::ops::Deref for Interned {
    type Target = str;
    fn deref(&self) -> &str {
        self.0
    }
}

impl std::fmt::Debug for Interned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.0, f)
    }
}

impl std::fmt::Display for Interned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl PartialEq<str> for Interned {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Interned {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Interned {
    fn eq(&self, other: &String) -> bool {
        self.0 == other.as_str()
    }
}

impl From<&str> for Interned {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for Interned {
    fn from(name: String) -> Self {
        Self::new(&name)
    }
}

impl<'de> Deserialize<'de> for Interned {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::new(&String::deserialize(d)?))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct FieldArray {
    #[serde(default)]
    pub text_items: Vec<String>,
    /// The raw elements as big-endian hex.
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Collection {
    #[serde(rename = "class")]
    pub class_name: String,
    pub key: String,
    pub parent: String,
    pub fields: BTreeMap<String, Field>,
    pub source: String,
    pub sha256: String,
}

/// skate_vita: the parsed collections are shared (`Arc`) and indexed once.
/// `field` used to scan every collection and format two `numeric_name` strings per
/// entry on every lookup (~130M allocations during a load), and each of the four
/// loaders re-parsed the 9.7 MB file. Lookups and results are unchanged; see `Index`.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "Raw")]
pub struct Collections {
    inner: Arc<Inner>,
}

#[derive(Debug, Deserialize)]
struct Raw {
    version: u32,
    collections: Vec<Collection>,
}

#[derive(Debug)]
struct Inner {
    version: u32,
    collections: Vec<Collection>,
    index: Index,
}

/// Exact replacements for the linear searches in `field`:
/// - `by_id[numeric_name(class)][numeric_name(key)]` is the FIRST collection in order
///   with those identities. The old match (`name == query || numeric_name(name) ==
///   numeric_name(query)`) is the same test, since equal names have equal ids.
/// - `field_ids[(i, numeric_name(field))]` is the rank (in `BTreeMap` order) of the first field
///   of collection `i` with that id: the old fallback `fields.iter().find(..)`.
/// skate_vita (memory): the ids are `numeric_key`s (what `numeric_name` formats) and a field is found by
/// its rank, not by a copy of its name (the index held two strings per field, 87,668 fields).
#[derive(Debug)]
struct Index {
    by_id: HashMap<u64, HashMap<u64, usize>>,
    field_ids: HashMap<(u32, u64), u32>,
}

/// skate_vita: the binary form of `skater-collections.json` (see `crate::binary`).
/// `COL2` (resync b3c9679) adds each field's array elements; a `COLL` pack must be rebaked.
/// `COL3` (resync to upstream 618e824) adds the arrays' raw elements (`FieldArray::items`, read by the
/// scoring tables); a `COL2` pack must be rebaked.
const BINARY_KIND: &[u8; 4] = b"COL3";

impl Raw {
    fn encode(&self) -> Vec<u8> {
        let mut w = crate::binary::Writer::new(BINARY_KIND);
        w.u32(self.version);
        w.u32(self.collections.len() as u32);
        for c in &self.collections {
            for s in [&c.class_name, &c.key, &c.parent, &c.source, &c.sha256] {
                w.str(s);
            }
            w.u32(c.fields.len() as u32);
            for (name, field) in &c.fields {
                w.str(name);
                w.str(&field.type_name);
                w.str(&field.data);
                match &field.array {
                    None => w.u32(u32::MAX),
                    Some(a) => {
                        w.u32(a.text_items.len() as u32);
                        for t in &a.text_items {
                            w.str(t);
                        }
                        w.u32(a.items.len() as u32);
                        for t in &a.items {
                            w.str(t);
                        }
                    }
                }
            }
        }
        w.bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut r = crate::binary::Reader::new(bytes, BINARY_KIND)?;
        let version = r.u32()?;
        let count = r.u32()? as usize;
        let mut collections = Vec::with_capacity(count);
        for _ in 0..count {
            let (class_name, key, parent, source, sha256) = (r.str()?, r.str()?, r.str()?, r.str()?, r.str()?);
            let mut fields = BTreeMap::new();
            for _ in 0..r.u32()? {
                let name = r.str()?;
                let (type_name, data) = (Interned::from(r.str()?), Interned::from(r.str()?));
                let array = match r.u32()? {
                    u32::MAX => None,
                    n => {
                        let text_items = (0..n).map(|_| r.str()).collect::<Result<_, _>>()?;
                        let items = (0..r.u32()?).map(|_| r.str()).collect::<Result<_, _>>()?;
                        Some(FieldArray { text_items, items })
                    }
                };
                let field = Field { type_name, data, array };
                fields.insert(name, field);
            }
            collections.push(Collection { class_name, key, parent, fields, source, sha256 });
        }
        r.done()?;
        Ok(Self { version, collections })
    }
}

/// skate_vita: the binary form of a `skater-collections.json` file, for `sim.pak`.
pub fn encode_json(json: &[u8]) -> Result<Vec<u8>, String> {
    let raw: Raw = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    Ok(raw.encode())
}

impl From<Raw> for Collections {
    fn from(raw: Raw) -> Self {
        let mut by_id: HashMap<u64, HashMap<u64, usize>> = HashMap::new();
        let mut field_ids = HashMap::with_capacity(raw.collections.iter().map(|c| c.fields.len()).sum());
        for (i, c) in raw.collections.iter().enumerate() {
            by_id
                .entry(crate::attrib_hash::numeric_key(&c.class_name))
                .or_default()
                .entry(crate::attrib_hash::numeric_key(&c.key))
                .or_insert(i);
            for (rank, key) in c.fields.keys().enumerate() {
                field_ids.entry((i as u32, crate::attrib_hash::numeric_key(key))).or_insert(rank as u32);
            }
        }
        Self {
            inner: Arc::new(Inner {
                version: raw.version,
                collections: raw.collections,
                index: Index { by_id, field_ids },
            }),
        }
    }
}

impl Collections {
    /// Replace one profile with a child of a base profile, without editing assets.
    /// skate_vita: upstream's `override_profile` on the shared, indexed form (copy on
    /// write, then the index is rebuilt as `From<Raw>` builds it).
    pub fn override_profile(&mut self, class: &str, target: &str, base: &str,
        fields: BTreeMap<String, Field>) -> Result<(), String> {
        let id = crate::attrib_hash::numeric_name(class);
        let base_id = crate::attrib_hash::numeric_name(base);
        if !self.inner.collections.iter().any(|c| crate::attrib_hash::numeric_name(&c.class_name)==id
            && crate::attrib_hash::numeric_name(&c.key)==base_id) { return Err("Missing base profile".into()); }
        let target_id = crate::attrib_hash::numeric_name(target);
        let mut collections = self.inner.collections.clone();
        collections.retain(|c| !(crate::attrib_hash::numeric_name(&c.class_name)==id
            && crate::attrib_hash::numeric_name(&c.key)==target_id));
        collections.push(Collection {class_name:class.into(),key:target.into(),parent:base.into(),
            fields,source:"host custom profile".into(),sha256:String::new()});
        *self = Self::from(Raw { version: self.inner.version, collections });
        Ok(())
    }

    pub fn entries(&self) -> &[Collection] {
        &self.inner.collections
    }

    /// Memoised per path (skate_vita): every loader asks for the same file, which
    /// does not change while the program runs.
    pub fn load(asset_root: &Path) -> Result<Self, String> {
        static CACHE: Mutex<Vec<(PathBuf, Collections)>> = Mutex::new(Vec::new());
        let path = asset_root.join("private/stock/skater-collections.json");
        if let Some((_, cached)) = CACHE.lock().unwrap().iter().find(|(p, _)| *p == path) {
            return Ok(cached.clone());
        }
        let bytes = crate::vfs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        // skate_vita: the baked binary form (sim.pak) or the JSON; same values.
        let data: Self = if crate::binary::kind(&bytes).is_some() {
            Self::from(Raw::decode(&bytes)?)
        } else {
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
        };
        if data.inner.version != 1 {
            return Err(format!(
                "Unsupported skater collections version {}",
                data.inner.version
            ));
        }
        let mut identities = std::collections::BTreeSet::new();
        for item in &data.inner.collections {
            if !identities.insert((crate::attrib_hash::numeric_name(&item.class_name), crate::attrib_hash::numeric_name(&item.key))) {
                return Err(format!(
                    "Duplicate collection {}/{}",
                    item.class_name, item.key
                ));
            }
        }
        CACHE.lock().unwrap().push((path, data.clone()));
        Ok(data)
    }

    pub fn field(&self, class: &str, key: &str, name: &str) -> Result<&Field, String> {
        // Converted vaults can retain numeric identities when debug names were
        // unavailable. Both spellings identify the same native AttribSys entry.
        let class_id = crate::attrib_hash::numeric_key(class);
        let field_id = crate::attrib_hash::numeric_key(name);
        let inner = &*self.inner;
        let class_index = inner.index.by_id.get(&class_id);
        let mut current = key;
        for _ in 0..=inner.collections.len() {
            let current_id = crate::attrib_hash::numeric_key(current);
            let index = class_index
                .and_then(|keys| keys.get(&current_id))
                .copied()
                .ok_or_else(|| format!("Missing stock collection {class}/{current}"))?;
            let item = &inner.collections[index];
            if let Some(field) = item.fields.get(name).or_else(|| {
                inner.index.field_ids
                    .get(&(index as u32, field_id))
                    .and_then(|&rank| item.fields.values().nth(rank as usize))
            }) {
                return Ok(field);
            }
            if item.parent.is_empty() {
                return Err(format!("Missing stock field {class}/{key}/{name}"));
            }
            current = &item.parent;
        }
        Err(format!("Cyclic stock collection inheritance {class}/{key}"))
    }

    pub fn float(&self, class: &str, key: &str, name: &str) -> Result<f32, String> {
        let field = self.field(class, key, name)?;
        if field.type_name != "EA::Reflection::Float" {
            return Err(format!("Expected float at {class}/{key}/{name}"));
        }
        let words = decode_words::<1>(&field.data)?;
        let value = f32::from_bits(words[0]);
        if !value.is_finite() {
            return Err(format!("Non-finite stock float {class}/{key}/{name}"));
        }
        Ok(value)
    }

    pub fn integer(&self, class: &str, key: &str, name: &str) -> Result<u32, String> {
        let field = self.field(class, key, name)?;
        if !matches!(
            field.type_name.as_str(),
            "EA::Reflection::Int32" | "EA::Reflection::UInt32"
        ) {
            return Err(format!("Expected integer at {class}/{key}/{name}"));
        }
        Ok(decode_words::<1>(&field.data)?[0])
    }

    pub fn boolean(&self, class: &str, key: &str, name: &str) -> Result<bool, String> {
        let field = self.field(class, key, name)?;
        if field.type_name != "EA::Reflection::Bool" {
            return Err(format!("Expected boolean at {class}/{key}/{name}"));
        }
        // Attributes store the byte followed by padding; layout bools may
        // contain only the byte. Reading a big-endian u32 gives the wrong bit.
        match field.data.get(..2) {
            Some("00") => Ok(false),
            Some("01") => Ok(true),
            _ => Err(format!("Invalid stock boolean {class}/{key}/{name}")),
        }
    }

    pub fn words<const N: usize>(
        &self,
        class: &str,
        key: &str,
        name: &str,
    ) -> Result<[u32; N], String> {
        decode_words(&self.field(class, key, name)?.data)
    }
}

fn decode_words<const N: usize>(text: &str) -> Result<[u32; N], String> {
    let hex: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.len() != N * 8 || !hex.is_ascii() {
        return Err(format!(
            "Expected {N} big-endian words, found {} bytes of hex",
            hex.len()
        ));
    }
    let mut words = [0; N];
    for (i, word) in words.iter_mut().enumerate() {
        *word = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16)
            .map_err(|e| format!("Invalid collection payload: {e}"))?;
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_export_names_resolve_readable_manual_settings() {
        // Identities and payload from the prepared vault; no asset IO or gameplay.
        let data: Collections = serde_json::from_value(serde_json::json!({
            "version": 1, "collections": [{
                "class": "anim_motion", "key": crate::attrib_hash::numeric_name("manual"),
                "parent": "", "source": "fixture", "sha256": "",
                "fields": { "Hash_A353CE1670D3AA40": { "type": "EA::Reflection::Float", "data": "3D23D70A" } }
            }]
        })).unwrap();
        assert_eq!(data.float("anim_motion", "manual", "manual_clamp_vel").unwrap().to_bits(), 0x3d23d70a);
        assert!(data.field("anim_motion", "manual", "missing").is_err());
    }

    #[test]
    fn readable_names_resolve_numeric_queries_through_inheritance() {
        let data: Collections = serde_json::from_value(serde_json::json!({
            "version": 1, "collections": [
                {"class":"example", "key":"child", "parent":crate::attrib_hash::numeric_name("base"),
                 "source":"fixture", "sha256":"", "fields":{}},
                {"class":"example", "key":"base", "parent":"", "source":"fixture", "sha256":"",
                 "fields":{"value":{"type":"EA::Reflection::Float", "data":"3F800000"}}}
            ]
        })).unwrap();
        assert_eq!(data.float(&crate::attrib_hash::numeric_name("example"),
            &crate::attrib_hash::numeric_name("child"), &crate::attrib_hash::numeric_name("value")).unwrap(), 1.0);
    }

    /// The original linear scan, kept as the reference for the index.
    fn field_linear<'a>(data: &'a Collections, class: &str, key: &str, name: &str) -> Result<&'a Field, String> {
        let class_id = crate::attrib_hash::numeric_name(class);
        let field_id = crate::attrib_hash::numeric_name(name);
        let mut current = key;
        for _ in 0..=data.inner.collections.len() {
            let current_id = crate::attrib_hash::numeric_name(current);
            let item = data.inner.collections.iter().find(|c| {
                (c.class_name == class || crate::attrib_hash::numeric_name(&c.class_name) == class_id)
                    && (c.key == current || crate::attrib_hash::numeric_name(&c.key) == current_id)
            }).ok_or_else(|| format!("Missing stock collection {class}/{current}"))?;
            if let Some(field) = item.fields.get(name).or_else(|| {
                item.fields.iter().find(|(key, _)| crate::attrib_hash::numeric_name(key) == field_id).map(|(_, f)| f)
            }) {
                return Ok(field);
            }
            if item.parent.is_empty() {
                return Err(format!("Missing stock field {class}/{key}/{name}"));
            }
            current = &item.parent;
        }
        Err(format!("Cyclic stock collection inheritance {class}/{key}"))
    }

    fn same(data: &Collections, class: &str, key: &str, name: &str) {
        match (data.field(class, key, name), field_linear(data, class, key, name)) {
            (Ok(a), Ok(b)) => assert!(std::ptr::eq(a, b), "{class}/{key}/{name}: different field"),
            (Err(a), Err(b)) => assert_eq!(a, b, "{class}/{key}/{name}"),
            (a, b) => panic!("{class}/{key}/{name}: indexed {a:?} vs linear {b:?}"),
        }
    }

    fn row(class: &str, key: &str, parent: &str, value: &str) -> serde_json::Value {
        serde_json::json!({"class": class, "key": key, "parent": parent, "source": "fixture", "sha256": "",
            "fields": {"value": {"type": "EA::Reflection::Float", "data": value}}})
    }

    #[test]
    fn index_matches_linear_scan_for_spellings_inheritance_and_errors() {
        let n = crate::attrib_hash::numeric_name;
        let data: Collections = serde_json::from_value(serde_json::json!({"version": 1, "collections": [
            row("example", "child", &n("base"), "00000001"),
            row("example", "base", "", "00000002"),
            row(&n("other"), "base", "", "00000003"),
            row("example", "loop_a", "loop_b", "00000004"),
            row("example", "loop_b", "loop_a", "00000005"),
        ]})).unwrap();
        for class in ["example", &n("example"), "other", &n("other"), "missing"] {
            for key in ["child", "base", &n("base"), "0x0", "loop_a", "missing"] {
                for name in ["value", &n("value"), "absent"] {
                    same(&data, class, key, name);
                }
            }
        }
    }

    #[test]
    fn index_keeps_the_first_duplicate_like_the_scan() {
        // load() rejects duplicates, but a directly deserialized vault may not.
        let n = crate::attrib_hash::numeric_name;
        let data: Collections = serde_json::from_value(serde_json::json!({"version": 1, "collections": [
            row("example", "dup", "", "00000001"),
            row(&n("example"), &n("dup"), "", "00000002"),
        ]})).unwrap();
        same(&data, "example", "dup", "value");
        assert_eq!(data.field("example", "dup", "value").unwrap().data, "00000001");
    }

    #[test]
    fn override_profile_resets_the_index() {
        let mut data: Collections = serde_json::from_value(serde_json::json!({"version": 1, "collections": [
            row("physics_mode", "easy", "", "00000001"),
            row("physics_mode", "test", "", "00000002"),
        ]})).unwrap();
        assert_eq!(data.field("physics_mode", "test", "value").unwrap().data, "00000002"); // builds the index
        let mut fields = BTreeMap::new();
        fields.insert("value".to_owned(), Field { type_name: "EA::Reflection::Float".into(), data: "00000009".into(), array: None });
        data.override_profile("physics_mode", "test", "easy", fields).unwrap();
        assert_eq!(data.field("physics_mode", "test", "value").unwrap().data, "00000009");
        same(&data, "physics_mode", "test", "value");
        same(&data, "physics_mode", "easy", "value");
    }

    /// Every (entry, field name) in the real converted vault, including names
    /// only reachable through inheritance, resolves identically.
    #[test]
    #[ignore = "requires SKATE3_ASSET_ROOT pointing to converted stock assets"]
    fn index_matches_linear_scan_on_private_collections() {
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
        let data = Collections::load(std::path::Path::new(&root)).unwrap();
        let mut names_by_class: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
        for item in &data.inner.collections {
            names_by_class.entry(crate::attrib_hash::numeric_name(&item.class_name)).or_default()
                .extend(item.fields.keys().cloned());
        }
        let mut checked = 0usize;
        for item in &data.inner.collections {
            for name in &names_by_class[&crate::attrib_hash::numeric_name(&item.class_name)] {
                same(&data, &item.class_name, &item.key, name);
                checked += 1;
            }
        }
        eprintln!("compared {checked} lookups over {} entries", data.inner.collections.len());
        assert!(checked > 0);
    }
}
