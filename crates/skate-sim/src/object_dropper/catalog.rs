//! The catalogue as the Vita bakes it (`vita-bake catalog`, `private/object-catalog/catalog.json`): the tree of
//! `fe_park_categories` and every item with its part, template, caps and costs.
use std::collections::BTreeMap;

/// Static piece or dynamic prop (the recipe a catalogue item comes from).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Part {
    Terrain,
    Props,
}

/// What an item costs once it is in the world, in the units the Vita budgets in (`catalog.json` `cost`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cost {
    pub triangles: u32,
    pub draws: u32,
    pub vertex_bytes: u32,
    /// Its textures in its part's shared table (a texture two items use is loaded once).
    pub textures: Vec<u16>,
    /// Template-space bounds (min, max).
    pub bounds: [[f32; 3]; 2],
    /// Collision triangles (what a placed piece adds to the world's collision).
    pub collision: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The recipe id (`<mod id>`).
    pub id: String,
    pub name: String,
    pub part: Part,
    /// Its sub category (`fe_park_categories` key).
    pub sub: String,
    /// Its template in the catalogue pack; None for the pieces without a model (teleporters, the spawner).
    pub template: Option<String>,
    pub teleporter: bool,
    /// The marker a piece without a model is drawn as (the park spawner, the teleporters: a `ribbon_indicator` row,
    /// retail `82866AD8`); None for the rest.
    pub marker: Option<String>,
    /// A park teleporter's role (0 start, 1 end) and channel (vault class `0xD01B99210BA7323C`).
    pub teleport: Option<(u8, u8)>,
    /// The recipe caps that are set (`Unlock`, `FPSCost`, `IgnoreFuser`, ...).
    pub caps: BTreeMap<String, String>,
    pub cost: Cost,
    /// Snap sockets in template space (terrain pieces: the model's type-8 locators): (group, frame as x, y, z axes and
    /// position). Group 0 is a snap point; the sockets sharing another group are one snap face.
    pub sockets: Vec<(u32, [[f32; 3]; 4])>,
}

impl Item {
    /// What the editor holds it as: its template, or the marker of a piece without a model; None when it has neither
    /// (it cannot be shown or held).
    pub fn held_as(&self) -> Option<String> {
        self.template.clone().or_else(|| self.marker.clone())
    }

    /// The recipe's `AllowCollision` cap (cap slot 1, set on 181 pieces: rails, ledges, hubbas, foliage, ...): it may go
    /// into other items with Collision on (NOT RETAIL YET: our reading of the cap's name; the designers' layouts sink
    /// these pieces' feet into pads, ramps and stairs).
    pub fn allows_collision(&self) -> bool {
        self.caps.get("AllowCollision").is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    /// The park's spawner (where the skater starts): moved and turned, never deleted, copied or grouped.
    pub fn is_spawner(&self) -> bool {
        self.sub == SPAWNER_SUB
    }

    /// The recipe's `AdvanceSnap` cap (retail 8287E4C0: set unless empty or 0xFF; the recipe writes `True` / `False`).
    pub fn advance_snap(&self) -> bool {
        self.caps.get("AdvanceSnap").is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub key: String,
    pub order: i32,
    /// The sub categories under a super category, in order.
    pub subs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    /// Super categories in retail's order (terrain, misc terrain, props).
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
    /// Each part's shared textures' sizes in bytes.
    pub texture_bytes: BTreeMap<Part, Vec<u32>>,
    /// Item index by id and by template (the first item of a template): the meter and the screen look items up for
    /// every piece of a park each tick.
    by_id: std::collections::HashMap<String, usize>,
    by_template: std::collections::HashMap<String, usize>,
}

/// The spawner's sub category (`SkatePark_Spawner`; retail lists it in no catalogue page).
pub const SPAWNER_SUB: &str = "spawner";

fn part(name: &str) -> Option<Part> {
    match name {
        "terrain" => Some(Part::Terrain),
        "props" => Some(Part::Props),
        _ => None,
    }
}

impl Catalog {
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let num = |x: &serde_json::Value| x.as_f64().unwrap_or(0.0);
        let mut categories: Vec<Category> = v["super_categories"]
            .as_array()
            .ok_or("catalog.json: no super_categories")?
            .iter()
            .map(|c| Category { key: c["key"].as_str().unwrap_or_default().to_owned(), order: num(&c["order"]) as i32, subs: Vec::new() })
            .collect();
        categories.sort_by_key(|c| c.order);
        let mut subs: Vec<(String, String, i32)> = v["sub_categories"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| (s["parent"].as_str().unwrap_or_default().to_owned(), s["key"].as_str().unwrap_or_default().to_owned(), num(&s["order"]) as i32))
            .collect();
        subs.sort_by_key(|s| s.2);
        for (parent, key, _) in subs {
            if let Some(c) = categories.iter_mut().find(|c| c.key == parent) {
                c.subs.push(key);
            }
        }
        let mut items = Vec::new();
        for i in v["items"].as_array().ok_or("catalog.json: no items")? {
            let Some(part) = i["kind"].as_str().and_then(part) else { continue };
            let c = &i["cost"];
            let corner = |k: usize| -> [f32; 3] { std::array::from_fn(|a| num(&c["bounds"][k][a]) as f32) };
            items.push(Item {
                id: i["id"].as_str().unwrap_or_default().to_owned(),
                name: i["name"].as_str().unwrap_or_default().to_owned(),
                part,
                sub: i["sub"].as_str().unwrap_or_default().to_owned(),
                template: i["template"].as_str().map(str::to_owned),
                teleporter: i["teleporter"].as_bool().unwrap_or(false),
                marker: i["indicator"].as_str().map(str::to_owned),
                teleport: i["teleport"].as_array().filter(|t| t.len() == 2).map(|t| (num(&t[0]) as u8, num(&t[1]) as u8)),
                caps: i["caps"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned())).collect(),
                sockets: i["sockets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| {
                        let f: Vec<f32> = s[1].as_array()?.iter().map(|v| num(v) as f32).collect();
                        (f.len() == 12).then(|| (s[0].as_u64().unwrap_or(0) as u32, std::array::from_fn(|r| [f[r * 3], f[r * 3 + 1], f[r * 3 + 2]])))
                    })
                    .collect(),
                cost: Cost {
                    triangles: num(&c["triangles"]) as u32,
                    draws: num(&c["draws"]) as u32,
                    vertex_bytes: num(&c["vertex_bytes"]) as u32,
                    textures: c["textures"].as_array().into_iter().flatten().map(|t| num(t) as u16).collect(),
                    bounds: [corner(0), corner(1)],
                    collision: c["collision"].as_u64().map_or(num(&c["triangles"]) as u32, |n| n as u32),
                },
            });
        }
        let mut texture_bytes = BTreeMap::new();
        for (name, p) in v["parts"].as_object().into_iter().flatten() {
            if let Some(part) = part(name) {
                texture_bytes.insert(part, p["texture_bytes"].as_array().into_iter().flatten().map(|b| num(b) as u32).collect());
            }
        }
        Ok(Self::new(categories, items, texture_bytes))
    }

    pub fn new(categories: Vec<Category>, items: Vec<Item>, texture_bytes: BTreeMap<Part, Vec<u32>>) -> Self {
        let mut by_id = std::collections::HashMap::new();
        let mut by_template = std::collections::HashMap::new();
        for (i, item) in items.iter().enumerate() {
            by_id.entry(item.id.clone()).or_insert(i);
            if let Some(t) = &item.template {
                by_template.entry(t.clone()).or_insert(i);
            }
        }
        Self { categories, items, texture_bytes, by_id, by_template }
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.by_id.get(id).map(|&i| &self.items[i])
    }

    /// The item a template belongs to (the first, when items share one).
    pub fn by_template(&self, template: &str) -> Option<&Item> {
        self.by_template.get(template).map(|&i| &self.items[i])
    }

    /// What the catalogue lists (retail `82880708` / `828803D8`, docs/object-dropper-survey.md "What the catalogue
    /// lists"): in a skate.Park (`park`) every item; in the open world the props of the DMO banks it loaded, Global and
    /// its district's (`district`, the map name); with `all` (ours, the "All objects" setting) every item anywhere.
    /// Terrain pieces show one entry per family (their style and size variants grouped, the first in recipe order).
    /// Items without a model or marker, the park's spawner, and empty sub categories are left out; markers show each. Returns (super category, [(sub category, item
    /// ids)]) in the tree's order.
    pub fn listing(&self, park: bool, district: &str, all: bool) -> Vec<(String, Vec<(String, Vec<String>)>)> {
        let mut out = Vec::new();
        for category in &self.categories {
            let mut subs = Vec::new();
            for sub in &category.subs {
                let mut families = std::collections::BTreeSet::new();
                let ids: Vec<String> = self
                    .in_sub(sub)
                    .filter(|i| i.held_as().is_some() && !i.is_spawner())
                    .filter(|i| all || park || (i.part == Part::Props && bank(&i.name).is_some_and(|b| b == "Global" || b.eq_ignore_ascii_case(district))))
                    .filter(|i| i.part == Part::Props || i.marker.is_some() || families.insert(family(&i.name)))
                    .map(|i| i.id.clone())
                    .collect();
                if !ids.is_empty() {
                    subs.push((sub.clone(), ids));
                }
            }
            if !subs.is_empty() {
                out.push((category.key.clone(), subs));
            }
        }
        out
    }

    /// The Size key's swap (retail `825D1948`: the size filter 1 -> 2 -> 3 -> 1): the next size of `id`'s family in its
    /// sub category after `id`'s own, skipping sizes the family lacks; None when the item has no size.
    pub fn next_size(&self, id: &str) -> Option<String> {
        let item = self.item(id)?;
        let size = size_of(&item.name);
        if size == 0 {
            return None;
        }
        let family = family(&item.name);
        let variants: Vec<&Item> = self.in_sub(&item.sub).filter(|i| i.template.is_some() && family_of(i, &family)).collect();
        (1..=3)
            .map(|k| (size + k - 1) % 3 + 1)
            .find_map(|s| variants.iter().find(|i| size_of(&i.name) == s))
            .map(|i| i.id.clone())
    }

    /// The Style page's swap (`825D0A98` -> `828772A8`): `id`'s variant in `style` (a `fe_park_itemtypes` item type:
    /// downtown, industrial, university), the same family and size in the same sub category; None when it has none.
    pub fn style_variant(&self, id: &str, style: &str) -> Option<String> {
        let item = self.item(id)?;
        style_of(&item.name)?;
        if style_of(&item.name) == Some(style) {
            return Some(id.to_owned());
        }
        let family = family(&item.name);
        let size = size_of(&item.name);
        self.in_sub(&item.sub)
            .find(|i| i.template.is_some() && style_of(&i.name) == Some(style) && size_of(&i.name) == size && family_of(i, &family))
            .map(|i| i.id.clone())
    }

    /// The items of a sub category, in recipe order.
    pub fn in_sub<'a>(&'a self, sub: &'a str) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().filter(move |i| i.sub == sub)
    }
}

/// The DMO bank (`worlddmo.big` stream) a catalogue prop's template is in, from the tag in its name
/// (`ap_DMO_glbl_WoodenPalette`): the 119 props' tags agree with the banks the streams hold (Global 66, DownTown 17,
/// Industrial 15 with one `lnd`, University 21), so Global plus a district is the 81 to 87 props retail's free play
/// lists.
pub fn bank(name: &str) -> Option<&'static str> {
    let mut parts = name.split('_').skip(1);
    if !parts.next()?.eq_ignore_ascii_case("dmo") {
        return None;
    }
    match parts.next()?.to_ascii_lowercase().as_str() {
        "glbl" => Some("Global"),
        "dt" => Some("DownTown"),
        "ind" | "lnd" => Some("Industrial"),
        "un" => Some("University"),
        _ => None,
    }
}

fn family_of(item: &Item, family_name: &str) -> bool {
    family(&item.name) == family_name
}

/// An item's style from its name's last token (`8287E980` against `fe_park_itemtypes`): downtown, industrial or
/// university.
pub fn style_of(name: &str) -> Option<&'static str> {
    match name.rsplit('_').next()?.to_ascii_lowercase().as_str() {
        "downtown" => Some("downtown"),
        "industrial" => Some("industrial"),
        "university" | "uni" => Some("university"),
        _ => None,
    }
}

/// An item's size from its name's third `_` token (retail `8287EBA8`, `GetSize`; the first `_` at index 2): `sm` /
/// `sml` 1, `md` / `med` 2, `lg` / `lrg` 3, else 0.
pub fn size_of(name: &str) -> u8 {
    if name.find('_') != Some(2) {
        return 0;
    }
    match name.split('_').nth(2).map(str::to_ascii_lowercase).as_deref() {
        Some("sm" | "sml") => 1,
        Some("md" | "med") => 2,
        Some("lg" | "lrg") => 3,
        _ => 0,
    }
}

/// A terrain piece's family: its name without the style (`_Downtown`, `_Industrial`, `_University`) and size (`_lrg`,
/// `_md`, `_sm`) parts the Quick Menu's Style and Size switch between (retail `8287E980` / `8287EBA8`).
pub fn family(name: &str) -> String {
    name.split('_')
        .filter(|part| !matches!(part.to_ascii_lowercase().as_str(), "downtown" | "industrial" | "university" | "lrg" | "md" | "sm"))
        .collect::<Vec<_>>()
        .join("_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, name: &str, part: Part, sub: &str) -> Item {
        Item { id: id.into(), name: name.into(), part, sub: sub.into(), template: Some(format!("T{id}")), teleporter: false, marker: None, teleport: None, caps: Default::default(), cost: Cost::default(), sockets: Vec::new() }
    }

    #[test]
    fn listing_follows_the_world_and_groups_variants() {
        let catalog = Catalog::new(
            vec![
                Category { key: "terrain".into(), order: 10, subs: vec!["street".into()] },
                Category { key: "props".into(), order: 30, subs: vec!["dmo_ramps".into()] },
            ],
            vec![
                item("1", "aa_padStreet_lrg_Downtown", Part::Terrain, "street"),
                item("2", "aa_padStreet_md_Industrial", Part::Terrain, "street"),
                item("3", "ab_kicker_University", Part::Terrain, "street"),
                item("4", "bo_DMO_UN_SOHOGarbageCan", Part::Props, "dmo_ramps"),
                item("5", "bn_DMO_Glbl_mediumQuarterPipeDeck", Part::Props, "dmo_ramps"),
            ],
            Default::default(),
        );
        // A park: everything, the pad's variants as one entry.
        let park = catalog.listing(true, "MegaPark", false);
        assert_eq!(park, vec![("terrain".into(), vec![("street".into(), vec!["1".into(), "3".into()])]), ("props".into(), vec![("dmo_ramps".into(), vec!["4".into(), "5".into()])])]);
        // The open world: Global's props and the district's.
        assert_eq!(catalog.listing(false, "DownTown", false), vec![("props".into(), vec![("dmo_ramps".into(), vec!["5".into()])])]);
        assert_eq!(catalog.listing(false, "University", false), vec![("props".into(), vec![("dmo_ramps".into(), vec!["4".into(), "5".into()])])]);
        // All objects: the park's list anywhere.
        assert_eq!(catalog.listing(false, "DownTown", true), park);
        assert_eq!((bank("ap_DMO_glbl_WoodenPalette"), bank("x_DMO_lnd_Pipe"), bank("aa_padStreet_lrg_Downtown")), (Some("Global"), Some("Industrial"), None));
        assert_eq!(family("ba_bigstairs_lrg_Downtown"), "ba_bigstairs");
        // Size: the pad's medium after its large, wrapping past the missing small.
        assert_eq!((size_of("aa_padStreet_lrg_Downtown"), size_of("ab_kicker_University")), (3, 0));
        assert_eq!(catalog.next_size("1").as_deref(), Some("2"));
        assert_eq!(catalog.next_size("2").as_deref(), Some("1"));
        assert_eq!(catalog.next_size("3"), None);
        // Style: the pad's Industrial variant of its large Downtown one does not exist (different size); the kicker's
        // style is University.
        assert_eq!((style_of("ab_kicker_University"), style_of("bo_DMO_UN_SOHOGarbageCan")), (Some("university"), None));
        assert_eq!(catalog.style_variant("1", "industrial"), None);
        assert_eq!(catalog.style_variant("2", "industrial").as_deref(), Some("2"));
    }
}
