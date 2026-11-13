//! The player's look from `cas.pak` (docs/customization-survey.md step 2): the created skater's worn items as one
//! skater mesh, built once when the outfit changes, so nothing here runs while skating.
//!
//! The worn slots go through retail's composition (`skate_core::cas_rules`) first; each slot of its result draws its
//! part with its material; a colour layer naming the item gives its tint (hair, skin), else the library's. A morph's weight is the record's value directly; the ethnicity targets are not in a record and stay 0.
//! Until a skater is saved, the player is retail's fallback character (`SavedRecipeFallback*`).
//!
//! The worn-in materials are drawn (`compose`'s `in_game`): the CAC screen will ask for the clean ones.
//!
//! Eyebrows and facial hair are in the head's base colour (one material per combination, picked by the composition).
//! Hair is cut by its opacity map on the second UV set at 0.5, double-sided (`skin_hair`).
//!
//! NOT RETAIL YET: the colour layers' second colour; the skin and cloth stamps (tattoos, graphics: nothing wears one
//! until those menus); glasses (retail's `cac_alpha` blends them; a material with alpha and no opacity map is cut at 0.5
//! here, which hides them); the skin shader's lighting. NPC skaters share this mesh, as they shared the stock one (the
//! Vita draws no roster), and show no hair while fading.
#![cfg_attr(not(target_os = "vita"), allow(dead_code))]
use serde_json::Value;
use skate_core::cac::CacData;
use skate_core::cas_rules::{Body, Camera, Catalog, Colour, MorphParam, Preset, TattooCategory, Vault, Zone};
use skate_core::skater_record::{Gender, SkaterRecord, Slot};
use std::collections::BTreeMap;
use vita_pack::cas::{merge, texture_from_pack, Look, Part, Piece};
use vita_pack::skater::{AlphaMode, Material};
use vita_pack::texture::Texture;

const LIBRARY: &str = "cas/library.json";
const RULES: &str = "cas/rules.bin";
const VAULT: &str = "cas/vault.json";
const FALLBACK_MALE: &str = "cas/recipes/SavedRecipeFallbackMale.bin";
const FALLBACK_FEMALE: &str = "cas/recipes/SavedRecipeFallbackFemale.bin";

/// Reads a pack file by its name in the pack.
type Read<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, String>;

/// Retail's fallback character.
fn fallback(read: Read, gender: Gender) -> Result<SkaterRecord, String> {
    let name = if gender == Gender::Female { FALLBACK_FEMALE } else { FALLBACK_MALE };
    SkaterRecord::from_retail(&read(name)?).map_err(|e| format!("{name}: {e}"))
}

/// The record's morph weights in the GLBs' target order (`morphs` of the library), by name through `morph_order`.
fn weights(record: &SkaterRecord, library: &Value) -> Vec<f32> {
    let names = library["morphs"].as_array().map_or(&[][..], Vec::as_slice);
    let order = library["morph_order"].as_array().map_or(&[][..], Vec::as_slice);
    names.iter().map(|name| order.iter().position(|o| o == name).and_then(|i| record.morphs.get(i)).copied().unwrap_or(0.0)).collect()
}

/// Builds looks, keeping what it has read: the customisation rebuilds the player at every change, and only new parts
/// and textures are read again.
pub struct Builder {
    library: Value,
    catalog: Catalog,
    parts: BTreeMap<String, Part>,
    textures: BTreeMap<String, Texture>,
}

impl Builder {
    pub fn new(read: Read) -> Result<Self, String> {
        Ok(Self {
            library: serde_json::from_slice(&read(LIBRARY)?).map_err(|e| format!("{LIBRARY}: {e}"))?,
            catalog: Catalog::from_bytes(&read(RULES)?).map_err(|e| format!("{RULES}: {e}"))?,
            parts: BTreeMap::new(),
            textures: BTreeMap::new(),
        })
    }

    fn texture(&mut self, read: Read, path: &str) -> Result<Texture, String> {
        if let Some(t) = self.textures.get(path) {
            return Ok(t.clone());
        }
        let t = texture_from_pack(&read(path)?).map_err(|e| format!("{path}: {e}"))?;
        self.textures.insert(path.to_owned(), t.clone());
        Ok(t)
    }

    /// The record's worn items as one mesh, `in_game` as [`Catalog::compose`]. An item the library lacks is left out
    /// and named in the log.
    pub fn assemble(&mut self, read: Read, record: &SkaterRecord, in_game: bool) -> Result<Look, String> {
        let weights = weights(record, &self.library);
        let list = self.catalog.compose(record, in_game);
        let mut pieces = Vec::new();
        let mut textures = Vec::new();
        let mut texture_of: BTreeMap<String, u16> = BTreeMap::new();
        // The tattoos and graphics (the record's graphics; the adjust index is the entry's slot byte - 1): the picture of
        // each, a tattoo's from the library's `tattoos` and a logo's from its `logos`. NOT RETAIL YET: a logo lies over
        // the cloth as the diffuse picture's alpha says (retail's cloth shader lays its decals over the same way).
        let mut decal_textures: [Option<Texture>; 5] = [None, None, None, None, None];
        for g in &record.graphics {
            let at = usize::from(g.flags[1]).wrapping_sub(1);
            if at >= 5 || record.adjust.words[at] == u32::MAX {
                continue;
            }
            let id = format!("{:016x}", g.values[1]);
            let table = if at == 1 || at == 2 { "tattoos" } else { "logos" };
            if let Some(path) = self.library[table][&id]["texture"].as_str().map(str::to_owned) {
                decal_textures[at] = Some(self.texture(read, &path)?);
            }
        }
        for slot in Slot::ALL {
            let item = list[slot as usize];
            if item.is_none() {
                continue;
            }
            let (model, material) = (format!("{:016x}", item.model), format!("{:016x}", item.material));
            let entry = self.library["materials"][&material].clone();
            let (Some(path), Some(texture)) = (self.library["models"][&model]["part"].as_str().map(str::to_owned), entry["texture"].as_str()) else {
                println!("CAC look: {} {model} / {material} is not in the library", slot.name());
                continue;
            };
            let part = match self.parts.get(&path) {
                Some(p) => p.clone(),
                None => {
                    let mut p = Part::from_pack(&read(&path)?).map_err(|e| format!("{path}: {e}"))?;
                    follow_the_hands(&mut p);
                    self.parts.insert(path.clone(), p.clone());
                    p
                }
            };
            let index = match texture_of.get(texture) {
                Some(&i) => i,
                None => {
                    let i = textures.len() as u16;
                    textures.push(self.texture(read, texture)?);
                    texture_of.insert(texture.to_owned(), i);
                    i
                }
            };
            let layer = record.colours.iter().find(|c| c.part == item).map(|c| c.index);
            // A material of a `default_` family (default_cloth, default_skin, default_skateboard: retail's shaders
            // without a modulate colour, the ones the catalog never marks IsColourizable) takes no colour layer: the
            // eyes and the deck are chosen by their colour (`eyes_of_colour`, `deck_of_colour`), their layer's colour is
            // not a tint. Tinting the eyes by their layer (a dark brown) drew them black.
            let colourizable = !entry["shader"].as_str().is_some_and(|f| f.starts_with("character.default_"));
            let tint = colour_of(record, slot, item).filter(|_| colourizable).or_else(|| {
                let t = entry["tint"].as_array()?;
                Some([t.first()?.as_f64()? as f32, t.get(1)?.as_f64()? as f32, t.get(2)?.as_f64()? as f32])
            });
            let [r, g, b] = tint.unwrap_or([1.0; 3]);
            let alpha = entry["alpha"].as_bool().unwrap_or(false);
            let opacity = entry["opacity"].as_str().map(|path| self.texture(read, path)).transpose()?;
            let mut lighting = lighting_of(&self.library, &entry);
            if let (Some(l), Some(index)) = (lighting.as_mut(), layer) {
                l.linear_tint = CLOTH_LAYERS.contains(&index);
            }
            pieces.push(Piece {
                part,
                opacity,
                lighting,
                decal: match slot {
                    // The upper tattoo lies on the bare top while the top is off, else on the arms' skin (NOT RETAIL
                    // YET: the same placement on the arm's own UV set; retail's target for a worn top is not traced).
                    Slot::OuterTorso if record.adjust.flag != 0 && decal_textures[1].is_some() => 2,
                    Slot::Arm if record.adjust.flag == 0 && decal_textures[1].is_some() => 2,
                    Slot::Leg if decal_textures[2].is_some() => 3,
                    // A shirt's graphic lies on the worn top, a board's and a hat's on theirs.
                    Slot::OuterTorso if record.adjust.flag == 0 && decal_textures[0].is_some() => 1,
                    Slot::SkateBoard if decal_textures[3].is_some() => 4,
                    Slot::Hat if decal_textures[4].is_some() => 5,
                    _ => 0,
                },
                weights: weights.clone(),
                material: Material {
                    name: format!("{}_{material}", slot.name()),
                    base_color: [r, g, b, 1.0],
                    texture: Some(index),
                    alpha_mode: match (alpha, slot) {
                        // Retail's cac_alpha blends the lenses; the hair's coverage is its opacity map.
                        (true, Slot::Glasses) => AlphaMode::Blend,
                        (true, _) => AlphaMode::Mask,
                        _ => AlphaMode::Opaque,
                    },
                    alpha_cutoff: 0.5,
                    double_sided: entry.get("opacity").is_some(),
                },
            });
        }
        let mut look = merge(pieces, textures)?;
        look.decals.textures = decal_textures;
        Ok(look)
    }

    /// What the customisation screen reads: this catalog, the vault rows and both fallback characters.
    pub fn screen_data(&self, read: Read) -> Result<CacData, String> {
        Ok(CacData { catalog: self.catalog.clone(), vault: vault(read)?, fallback: [fallback(read, Gender::Female)?, fallback(read, Gender::Male)?] })
    }
}

/// The clothes' colour layers (hat 0, outer torso 9, socks 10, pants 12; 82538060): their first vector is the cloth
/// shaders' linear `i_modulateColor1` (cacstamp_shift / cac_cloth / cacstamp_cloth `defaultPS`: the squared diffuse
/// times it, the decals over it). [code]
const CLOTH_LAYERS: [u8; 4] = [0, 9, 10, 12];

/// A part's vertices skinned to a helper bone (`*_REPARENTED`: the rings use `RIGHTHAND_REPARENTED`, retail's hand target
/// under the board root) follow the real bone the helper is bound on instead: the simulation poses the helpers as the
/// animation's targets, not as the solved hand, so a ring on the helper floated beside the hand. The bone is the one
/// whose bind pose is the helper's (its inverse bind nearest), not the one its name says: `RIGHTHAND_REPARENTED` is bound
/// at the LEFT hand (the GLBs' inverse binds; the rings are modelled there), and following `RIGHTHAND` swung the ring a
/// body's width away (owner, device). With the same bind the skinning keeps every vertex in place at rest.
fn follow_the_hands(part: &mut Part) {
    use vita_pack::skater::{JOINTS, STRIDE};
    let joints = &part.mesh.joints;
    let distance = |a: &[f32; 16], b: &[f32; 16]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>();
    let base: Vec<u8> = joints
        .iter()
        .enumerate()
        .map(|(i, j)| {
            if !j.name.ends_with("_REPARENTED") {
                return i as u8;
            }
            joints
                .iter()
                .enumerate()
                .filter(|(_, k)| !k.name.ends_with("_REPARENTED"))
                .min_by(|a, b| distance(&a.1.inverse_bind, &j.inverse_bind).total_cmp(&distance(&b.1.inverse_bind, &j.inverse_bind)))
                .map_or(i as u8, |(k, _)| k as u8)
        })
        .collect();
    if base.iter().enumerate().all(|(i, &b)| usize::from(b) == i) {
        return;
    }
    for v in part.mesh.vertex_data.chunks_exact_mut(STRIDE) {
        for j in &mut v[JOINTS..JOINTS + 4] {
            *j = base.get(usize::from(*j)).copied().unwrap_or(*j);
        }
    }
}

/// The colour layer of a drawn item: the layer naming it, else its slot's (hair 1, arm 2, leg 3, head 4). The
/// composition swaps the head for the material of the hair colour (the brows are in its texture) and the skin tone, so
/// the layer's part can name the head worn before.
fn colour_of(record: &SkaterRecord, slot: Slot, item: skate_core::skater_record::Item) -> Option<[f32; 3]> {
    if let Some(c) = record.colours.iter().find(|c| c.part == item) {
        return Some(c.primary);
    }
    let index = match slot {
        Slot::Hair => 1,
        Slot::Arm => 2,
        Slot::Leg => 3,
        Slot::Rostral => 4,
        _ => return None,
    };
    record.colours.iter().find(|c| c.index == index && record.colour_keys[usize::from(index)] != u64::MAX).map(|c| c.primary)
}

/// A material's retail shading: its family (`shader`) and the family's nine rows (`shaders` in the library).
fn lighting_of(library: &Value, material: &Value) -> Option<vita_pack::cas::Lighting> {
    let shader = material["shader"].as_str()?;
    let rows = library["shaders"][shader].as_array().filter(|r| r.len() == 9)?;
    let mut out = [[0f32; 4]; 9];
    for (row, values) in out.iter_mut().zip(rows) {
        for (c, x) in row.iter_mut().zip(values.as_array()?) {
            *c = x.as_f64()? as f32;
        }
    }
    let hair = shader.starts_with("character.hair");
    Some(vita_pack::cas::Lighting { rows: out, hair, linear_tint: false })
}

impl Builder {
    /// The characters' ambient light (the library's `default_sh`), when the library has it.
    pub fn ambient(&self) -> Option<[[f32; 3]; 9]> {
        let sh = self.library["default_sh"].as_array().filter(|a| a.len() == 9)?;
        let mut out = [[0f32; 3]; 9];
        for (c, v) in out.iter_mut().zip(sh) {
            for (x, y) in c.iter_mut().zip(v.as_array()?) {
                *x = y.as_f64()? as f32;
            }
        }
        Some(out)
    }
}

/// The record's worn items as one mesh, as in the game.
pub fn assemble(read: Read, record: &SkaterRecord) -> Result<Look, String> {
    Builder::new(read)?.assemble(read, record, true)
}

/// `cas/vault.json` (vita-bake `cas`): the colour zones, morph limits, presets and cameras.
pub fn vault(read: Read) -> Result<Vault, String> {
    let v: Value = serde_json::from_slice(&read(VAULT)?).map_err(|e| format!("{VAULT}: {e}"))?;
    let key = |v: &Value| v.as_str().and_then(|t| u64::from_str_radix(t, 16).ok()).unwrap_or(0);
    let vec3 = |v: &Value| [0, 1, 2].map(|i| v[i].as_f64().unwrap_or(0.0) as f32);
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    Ok(Vault {
        zones: list(&v["zones"])
            .iter()
            .map(|z| Zone {
                key: key(&z["key"]),
                slot: z["slot"].as_u64().unwrap_or(0) as u32,
                colours: list(&z["colours"])
                    .iter()
                    .map(|c| Colour { key: key(&c["key"]), primary: vec3(&c["row0"]), swatch: vec3(&c["swatch"]), secondary: vec3(&c["row20"]), tag: c["tag"].as_str().unwrap_or("").to_owned() })
                    .collect(),
            })
            .collect(),
        morphs: list(&v["morphs"])
            .iter()
            .map(|m| MorphParam {
                key: m["key"].as_str().unwrap_or("").to_owned(),
                index: m["index"].as_u64().unwrap_or(0) as usize,
                min: m["min"].as_f64().unwrap_or(0.0) as f32,
                max: m["max"].as_f64().unwrap_or(1.0) as f32,
            })
            .collect(),
        presets: list(&v["presets"])
            .iter()
            .map(|p| Preset {
                key: p["key"].as_str().unwrap_or("").to_owned(),
                male: p["male"].as_bool().unwrap_or(false),
                number: p["number"].as_u64().unwrap_or(0) as u32,
                label: p["label"].as_str().unwrap_or("").to_owned(),
                morphs: list(&p["morphs"]).iter().map(|m| (m[0].as_str().unwrap_or("").to_owned(), m[1].as_f64().unwrap_or(0.0) as f32)).collect(),
                colours: list(&p["colours"]).iter().map(|c| (key(&c[0]), key(&c[1]))).collect(),
            })
            .collect(),
        cameras: list(&v["cameras"])
            .iter()
            .map(|c| Camera { key: c["key"].as_str().unwrap_or("").to_owned(), eye: vec3(&c["eye"]), target: vec3(&c["target"]), fov: c["fov"].as_f64().unwrap_or(35.0) as f32 })
            .collect(),
        bodies: list(&v["bodies"])
            .iter()
            .map(|b| Body {
                key: b["key"].as_str().unwrap_or("").to_owned(),
                position: vec3(&b["position"]),
                heading: b["heading"].as_f64().unwrap_or(0.0) as f32,
                turn: b["turn"].as_f64().unwrap_or(0.0) as f32,
                anim: b["anim"].as_str().unwrap_or("").to_owned(),
            })
            .collect(),
        postures: list(&v["postures"]).iter().map(|p| (p["key"].as_str().unwrap_or("").to_owned(), p["anim"].as_str().unwrap_or("").to_owned())).collect(),
        hidden_clothing: list(&v["hidden_unlocks"]["clothing"]).iter().filter_map(|n| n.as_str().map(str::to_owned)).collect(),
        hidden_sponsors: list(&v["hidden_unlocks"]["sponsors"]).iter().filter_map(|n| n.as_str().map(str::to_owned)).collect(),
        tattoo_categories: list(&v["tattoo_categories"])
            .iter()
            .map(|c| TattooCategory { text: c["text"].as_str().unwrap_or("").to_owned(), untagged: c["untagged"].as_bool().unwrap_or(false) })
            .collect(),
        stamp_zones: list(&v["stamp_zones"])
            .iter()
            .filter_map(|z| Some((z["key"].as_str()?.to_owned(), z["camera"].as_str()?.to_owned())))
            .collect(),
    })
}

/// The record the player wears: the first saved skater in use, else retail's male fallback.
pub fn player_record(read: Read, skaters: &crate::skaters::Skaters) -> Result<SkaterRecord, String> {
    match skaters.iter().flatten().find(|r| crate::skaters::in_use(r)) {
        Some(r) => Ok(r.clone()),
        None => fallback(read, Gender::Male),
    }
}

/// How many looks the NPC skaters share (each about 0.7 MB on the GPU, a second or so of the boot on the device).
pub const NPC_LOOKS: usize = 3;
/// The disc's saved characters (`SavedRecipe0` to `SavedRecipe10`).
const SAVED_RECIPES: usize = 11;

/// The NPC skaters' looks: the disc's saved characters (retail's characters other than the player come from the
/// `SavedRecipe%s` pool, 8257E3E0 with the vault's count), the first `count` that assemble, in order. An NPC wears one by
/// its character (`NpcPose::look`). Empty when the pack lacks them; the NPCs then wear the player's mesh, as before.
pub fn npc_looks(builder: &mut Builder, read: Read, count: usize) -> Vec<Look> {
    let mut out = Vec::new();
    for i in 0..SAVED_RECIPES {
        if out.len() >= count {
            break;
        }
        let name = format!("cas/recipes/SavedRecipe{i}.bin");
        let record = match read(&name).and_then(|b| SkaterRecord::from_retail(&b).map_err(|e| e.to_string())) {
            Ok(r) => r,
            Err(e) => {
                println!("CAC NPC look {name}: {e}");
                continue;
            }
        };
        match builder.assemble(read, &record, true) {
            Ok(look) => out.push(look),
            Err(e) => println!("CAC NPC look {name}: {e}"),
        }
    }
    out
}

/// The player's mesh from `cas.pak`, for the renderer, and the NPC skaters' looks.
#[cfg(target_os = "vita")]
pub fn load(skaters: &crate::skaters::Skaters) -> Result<(Look, SkaterRecord, Builder, Vec<Look>), String> {
    let started = std::time::Instant::now();
    crate::packs::mount_on_card(&format!("{}/cas.pak", crate::packs::DIR), crate::packs::ROOT)?;
    let record = player_record(&read, skaters)?;
    let mut builder = Builder::new(&read)?;
    let look = builder.assemble(&read, &record, true)?;
    let skater = &look.skater;
    println!(
        "CAC look: {} parts, {} vertices, {} textures ({} KiB) in {} ms",
        skater.primitives.len(),
        skater.vertex_count(),
        skater.textures.len(),
        skater.textures.iter().map(vita_pack::texture::Texture::bytes).sum::<usize>() / 1024,
        started.elapsed().as_millis()
    );
    let started = std::time::Instant::now();
    let npcs = npc_looks(&mut builder, &read, NPC_LOOKS);
    println!("CAC NPC looks: {} ({} vertices) in {} ms", npcs.len(), npcs.iter().map(|l| l.skater.vertex_count()).sum::<usize>(), started.elapsed().as_millis());
    Ok((look, record, builder, npcs))
}

/// Reads from the mounted `cas.pak`.
#[cfg(target_os = "vita")]
pub fn read(p: &str) -> Result<Vec<u8>, String> {
    skate_data::vfs::read(&std::path::Path::new(crate::packs::ROOT).join(p)).map_err(|e| format!("{p}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use skate_core::skater_record::{ColourLayer, Item};
    use std::collections::{BTreeMap as Fields, HashMap};
    use vita_pack::cas::{texture_to_pack, Morph};
    use vita_pack::skater::{Joint, Primitive, Skater, Vertex};
    use vita_pack::texture::{Format, Texture};

    const HEAD: Item = Item { model: 0xA, material: 0xB };
    const HAIR: Item = Item { model: 0xC, material: 0xD };
    const TEE: Item = Item { model: 0xE, material: 0xF };

    fn part(morphs: Vec<Morph>) -> Vec<u8> {
        let mut vertex_data = Vec::new();
        for i in 0..3 {
            Vertex { position: [i as f32, 0.0, 0.0], normal: [0, 127, 0], uv: [0.0; 2], joints: [0; 4], weights: [255, 0, 0, 0] }.encode(&mut vertex_data);
        }
        let identity: [f32; 16] = std::array::from_fn(|i| f32::from(u8::from(i % 5 == 0)));
        let mesh = Skater {
            joints: vec![Joint { name: "HIPS".into(), parent: None, inverse_bind: identity }],
            vertex_data,
            indices: vec![0, 1, 2],
            primitives: vec![Primitive { first_index: 0, index_count: 3, material: 0 }],
            materials: vec![Material { name: "glb".into(), base_color: [1.0; 4], texture: None, alpha_mode: AlphaMode::Opaque, alpha_cutoff: 0.5, double_sided: false }],
            textures: Vec::new(),
        };
        Part { mesh, uv1: Vec::new(), morphs }.to_pack()
    }

    fn rules() -> Vec<u8> {
        let empty = Fields::new();
        // Hair of a type other than `hat` stays through the composition without a hat.
        let hair = Fields::from([("HairModelType".to_owned(), "full".to_owned())]);
        Catalog::build(&[(Slot::Rostral, HEAD, empty.clone()), (Slot::Hair, HAIR, hair), (Slot::OuterTorso, TEE, empty)], &[]).to_bytes()
    }

    fn pack() -> HashMap<String, Vec<u8>> {
        let texture = texture_to_pack(&Texture { width: 1, height: 1, format: Format::Rgba8, mips: vec![vec![9; 4]] });
        let library = json!({
            "morphs": ["fat", "face", "global_ethnicity_african"],
            "morph_order": ["fat", "thin", "face"],
            "models": {
                "000000000000000a": { "part": "p/head" },
                "000000000000000c": { "part": "p/hair" },
                "000000000000000e": { "part": "p/tee" },
            },
            "materials": {
                "000000000000000b": { "tint": [1.0, 1.0, 1.0], "alpha": false, "texture": "t/skin" },
                "000000000000000d": { "tint": [1.0, 1.0, 1.0], "alpha": true, "texture": "t/hair", "opacity": "t/op" },
                "000000000000000f": { "tint": [0.5, 0.25, 1.0], "alpha": false, "texture": "t/skin" },
            },
        });
        let face = Morph { vertices: vec![0], positions: vec![[0.0, 1.0, 0.0]], normals: vec![[0.0; 3]] };
        let none = Morph { vertices: Vec::new(), positions: Vec::new(), normals: Vec::new() };
        HashMap::from([
            (LIBRARY.to_owned(), serde_json::to_vec(&library).unwrap()),
            ("p/head".to_owned(), part(vec![none.clone(), face, none.clone()])),
            ("p/hair".to_owned(), part(vec![none.clone(), none.clone(), none])),
            ("p/tee".to_owned(), part(Vec::new())),
            ("t/skin".to_owned(), texture.clone()),
            ("t/op".to_owned(), texture.clone()),
            ("t/hair".to_owned(), texture),
            (RULES.to_owned(), rules()),
        ])
    }

    #[test]
    fn worn_items_make_one_mesh_with_tints_alpha_and_morphs() {
        let files = pack();
        let read = |p: &str| files.get(p).cloned().ok_or_else(|| format!("{p}: missing"));
        let mut record = SkaterRecord::cleared();
        record.worn[Slot::Rostral as usize] = HEAD;
        record.worn[Slot::Hair as usize] = HAIR;
        record.worn[Slot::OuterTorso as usize] = TEE;
        // Not in the library: left out.
        record.worn[Slot::Hat as usize] = Item { model: 0x99, material: 0x98 };
        record.morphs = vec![0.0, 0.0, 0.25];
        record.colours.push(ColourLayer { index: 2, primary: [0.72, 0.57, 0.49], secondary: [0.0; 3], part: HEAD });
        let s = assemble(&read, &record).unwrap().skater;
        // In slot order: hair, head, then the outer torso.
        let names: Vec<&str> = s.materials.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Hair_000000000000000d", "Rostral_000000000000000b", "OuterTorso_000000000000000f"]);
        // Two parts share the skin texture.
        assert_eq!(s.textures.len(), 2);
        assert_eq!(s.materials[1].base_color, [0.72, 0.57, 0.49, 1.0]);
        assert_eq!(s.materials[2].base_color, [0.5, 0.25, 1.0, 1.0]);
        assert_eq!((s.materials[0].alpha_mode, s.materials[0].double_sided), (AlphaMode::Mask, true));
        assert_eq!((s.materials[1].alpha_mode, s.materials[1].double_sided), (AlphaMode::Opaque, false));
        // The face slider by name: the record's third value on the GLB's second target, on the head's first vertex.
        assert_eq!(Vertex::decode(&s.vertex_data[3 * vita_pack::skater::STRIDE..]).position, [0.0, 0.25, 0.0]);
    }

    #[test]
    fn a_swapped_head_keeps_the_skins_colour() {
        let mut record = SkaterRecord::cleared();
        record.colour_keys[4] = 7;
        // The skin layer names the head worn before the composition swapped it.
        record.colours.push(ColourLayer { index: 4, primary: [0.72, 0.57, 0.49], secondary: [0.0; 3], part: Item { model: 0xA, material: 0x1 } });
        record.colours.push(ColourLayer { index: 1, primary: [0.1, 0.1, 0.1], secondary: [0.0; 3], part: HAIR });
        assert_eq!(colour_of(&record, Slot::Rostral, HEAD), Some([0.72, 0.57, 0.49]));
        assert_eq!(colour_of(&record, Slot::Hair, HAIR), Some([0.1, 0.1, 0.1]));
        assert_eq!(colour_of(&record, Slot::OuterTorso, TEE), None);
    }

    #[test]
    fn a_saved_skater_comes_before_the_fallback() {
        let read = |p: &str| Err::<Vec<u8>, _>(format!("{p}: missing"));
        let mut saved = SkaterRecord::cleared();
        assert!(player_record(&read, &[Some(saved.clone()), None, None, None, None]).is_err(), "not in use: the fallback is read");
        saved.worn[Slot::Rostral as usize] = HEAD;
        assert_eq!(player_record(&read, &[None, Some(saved.clone()), None, None, None]).unwrap(), saved);
    }

    /// A saved character's composed block (82C27388): each group's slot, models and their materials, LOD 0.
    fn composed(bytes: &[u8]) -> BTreeMap<String, Vec<(u64, Vec<u64>)>> {
        struct Be<'a>(&'a [u8], usize);
        impl Be<'_> {
            fn take(&mut self, n: usize) -> &[u8] {
                self.1 += n;
                &self.0[self.1 - n..self.1]
            }
            fn u32(&mut self) -> usize {
                u32::from_be_bytes(self.take(4).try_into().unwrap()) as usize
            }
            fn u64(&mut self) -> u64 {
                u64::from_be_bytes(self.take(8).try_into().unwrap())
            }
            fn string(&mut self) -> String {
                let n = self.u32();
                String::from_utf8_lossy(self.take(n)).into_owned()
            }
        }
        // Past the version, its length, the section's version, then the block's own length.
        let n = u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut r = Be(&bytes[16..16 + n], 0);
        assert_eq!(r.u32(), 7);
        r.string();
        let groups = {
            r.u32();
            r.u32();
            r.u32();
            r.u32()
        };
        let mut out = BTreeMap::new();
        for _ in 0..groups {
            let slot = r.string();
            let mut models = Vec::new();
            for _ in 0..r.u32() {
                let model = r.u64();
                let mut first = None;
                for _ in 0..r.u32() {
                    r.take(17);
                    let mut materials = Vec::new();
                    for _ in 0..r.u32() {
                        for _ in 0..r.u32() {
                            materials.push(r.u64());
                            for _ in 0..r.u32() {
                                r.string();
                                r.u64();
                            }
                        }
                    }
                    first.get_or_insert(materials);
                }
                models.push((model, first.unwrap_or_default()));
            }
            out.insert(slot, models);
        }
        out
    }

    /// The composition against retail's own: every recipe on the disc, from its worn slots to its composed block.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn the_composition_matches_every_recipe_on_the_disc() {
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).unwrap();
            pack[*at as usize..*at as usize + len].to_vec()
        };
        let catalog = Catalog::from_bytes(&read(RULES)).unwrap();
        let mut wrong = 0;
        for (name, _, _) in index.iter().filter(|e| e.0.starts_with("cas/recipes/")) {
            let bytes = read(name);
            let record = SkaterRecord::from_retail(&bytes).unwrap();
            let want = composed(&bytes);
            let list = catalog.compose(&record, true);
            for slot in Slot::ALL {
                let item = list[slot as usize];
                let expected = want.get(slot.name());
                let ok = match expected {
                    None => item.is_none(),
                    Some(models) => models.iter().any(|(m, mats)| *m == item.model && mats.contains(&item.material)),
                };
                // The demo disc's characters keep a clean deck under worn-in shoes and wheels; the 13 others have none.
                let demo_deck = name.contains("DemoRecipe") && slot == Slot::SkateBoard;
                if !ok && !demo_deck {
                    wrong += 1;
                    println!("{name} {}: worn {:016x}/{:016x} composed to {:016x}/{:016x}, retail {:x?}", slot.name(), record.worn[slot as usize].model, record.worn[slot as usize].material, item.model, item.material, expected);
                }
            }
        }
        assert_eq!(wrong, 0);
    }

    /// `SKATEVITA` at the folder holding the real `cas.pak`: retail's fallback characters assemble.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn the_fallback_characters_assemble_from_the_real_pack() {
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        for gender in [Gender::Male, Gender::Female] {
            let record = fallback(&read, gender).unwrap();
            assert_eq!(record.gender, gender);
            assert_eq!((record.stance, record.style, record.posture), (0, 3, 0));
            let look = assemble(&read, &record).unwrap();
            assert_eq!(look.uv1.len(), look.skater.vertex_count());
            let hair = look.skater.materials.iter().position(|m| m.name.starts_with("Hair_")).unwrap();
            assert!(look.opacity[hair].is_some(), "hair has its opacity map");
            // Retail's shading on every part, hair with its own response; the ambient light is in the library.
            assert!(look.lighting.iter().all(Option::is_some), "{:?}", look.lighting.iter().filter(|l| l.is_none()).count());
            assert!(look.lighting[hair].is_some_and(|l| l.hair));
            assert!(Builder::new(&read).unwrap().ambient().is_some());
            let s = look.skater;
            let bytes: usize = s.textures.iter().map(Texture::bytes).sum();
            println!("{gender:?}: {} parts, {} vertices, {} textures, {} KiB", s.primitives.len(), s.vertex_count(), s.textures.len(), bytes / 1024);
            for m in &s.materials {
                println!("  {} tint {:?} {:?}", m.name, &m.base_color[..3], m.alpha_mode);
            }
            assert_eq!(s.joints.len(), 35);
            assert!(s.primitives.len() >= 10);
        }
    }

    /// `SKATEVITA` at the folder holding the real `cas.pak`: a ring follows the hand bone, the eyes keep their own colour
    /// whatever their layer says, and the NPC looks assemble from the disc's saved characters.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn rings_follow_the_hand_eyes_stay_clear_and_npcs_get_looks() {
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let mut builder = Builder::new(&read).unwrap();
        let mut record = fallback(&read, Gender::Male).unwrap();
        // A ring (the library's first male one) and a dark layer over the eyes.
        let ring = builder.library["models"].as_object().unwrap().iter().find(|(_, m)| m["slot"] == "Accessory" && m["flags"]["Gender"] == "male").map(|(k, m)| (u64::from_str_radix(k, 16).unwrap(), u64::from_str_radix(m["materials"][0].as_str().unwrap(), 16).unwrap())).unwrap();
        record.worn[Slot::Accessory as usize] = Item { model: ring.0, material: ring.1 };
        let eyes = record.worn[Slot::Organ as usize];
        record.colours.push(ColourLayer { index: 6, primary: [0.05, 0.03, 0.02], secondary: [0.0; 3], part: eyes });
        let look = builder.assemble(&read, &record, true).unwrap();
        let eye = look.skater.materials.iter().find(|m| m.name.starts_with("Organ_")).expect("eyes drawn");
        assert_eq!(&eye.base_color[..3], &[1.0, 1.0, 1.0], "the eyes are not tinted by their colour layer");
        let ring_material = look.skater.materials.iter().position(|m| m.name.starts_with("Accessory_")).expect("ring drawn");
        let joints = &look.skater.joints;
        let helper = joints.iter().find(|j| j.name == "RIGHTHAND_REPARENTED").unwrap();
        let mut bones = std::collections::BTreeSet::new();
        for p in look.skater.primitives.iter().filter(|p| usize::from(p.material) == ring_material) {
            for &i in &look.skater.indices[p.first_index as usize..(p.first_index + p.index_count) as usize] {
                let v = vita_pack::skater::Vertex::decode(&look.skater.vertex_data[usize::from(i) * vita_pack::skater::STRIDE..]);
                bones.extend(v.joints.iter().zip(v.weights).filter(|(_, w)| *w > 0).map(|(j, _)| joints[usize::from(*j)].name.clone()));
            }
        }
        // The helper is bound at the left hand: the ring follows LEFTHAND, whose bind is the helper's (so it rests in place).
        assert_eq!(bones.into_iter().collect::<Vec<_>>(), ["LEFTHAND"]);
        let left = joints.iter().find(|j| j.name == "LEFTHAND").unwrap();
        assert!(left.inverse_bind.iter().zip(&helper.inverse_bind).all(|(a, b)| (a - b).abs() < 0.01));
        let looks = npc_looks(&mut builder, &read, NPC_LOOKS);
        assert_eq!(looks.len(), NPC_LOOKS);
        let player = builder.assemble(&read, &fallback(&read, Gender::Male).unwrap(), true).unwrap();
        assert!(looks.iter().all(|l| l.skater.vertex_data != player.skater.vertex_data), "the NPCs do not wear the player's outfit");
    }

    /// `SKATEVITA` at the folder holding the real `cas.pak`: every Slappy's page of both fallback characters lists
    /// rows, each tile's thumbnail is in the pack, the locks follow a fresh profile and unlock-all opens them.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn slappys_pages_list_rows_whose_thumbnails_are_in_the_pack() {
        use skate_core::cac::{Cac, MENU_SLAPPYS, MENU_THUMBNAILS};
        use skate_core::unlocks::UnlockStore;
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let names: std::collections::BTreeSet<&str> = index.iter().map(|e| e.0.as_str()).collect();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let data = std::sync::Arc::new(Builder::new(&read).unwrap().screen_data(&read).unwrap());
        for gender in [0, 1] {
            for unlock_all in [false, true] {
                let mut store = UnlockStore::fresh_profile();
                store.unlock_all = unlock_all;
                let mut c = Cac::open(data.fallback[gender].clone(), &store, Some(data.clone()));
                c.select(1);
                assert_eq!(c.menu(), MENU_SLAPPYS);
                // Slappy's nine categories, then Accessories' four and Skateboards' three.
                let pages: Vec<(Option<i32>, i32)> = (0..9).map(|r| (None, r)).chain((0..4).map(|r| (Some(9), r))).chain((0..3).map(|r| (Some(10), r))).collect();
                let (mut rows, mut locked) = (0, 0);
                for (sub, row) in pages {
                    if let Some(s) = sub {
                        c.select(s);
                    }
                    let started = std::time::Instant::now();
                    c.select(row);
                    let ms = started.elapsed().as_secs_f32() * 1000.0;
                    assert_eq!(c.menu(), MENU_THUMBNAILS);
                    let title = c.title();
                    let n = c.row_count();
                    let page_locked = (0..n as i32).filter(|&i| c.row_kind(i) == "locked").count();
                    for i in 0..n as i32 {
                        let key = skate_ui::fe::retail_menu_movies::image_key(&c.thumbnail(i));
                        assert!(names.contains(format!("cas/thumbs/{key}.tex").as_str()), "{title} tile {i}: {key}");
                    }
                    println!("gender {gender} unlock-all {unlock_all}: {title}: {n} tiles, {page_locked} locked, built in {ms:.2} ms");
                    assert!(n > 1, "{title}");
                    rows += n;
                    locked += page_locked;
                    c.key(skate_core::cac::Key::Back);
                    if sub.is_some() {
                        c.key(skate_core::cac::Key::Back);
                    }
                    assert_eq!(c.menu(), MENU_SLAPPYS);
                }
                println!("gender {gender} unlock-all {unlock_all}: {rows} tiles, {locked} locked");
                if unlock_all {
                    assert_eq!(locked, 0, "unlock-all opens every lock");
                } else {
                    assert!(locked > 0, "a fresh profile has locked clothes");
                }
            }
        }
    }

    /// `SKATEVITA` at the folder holding the real `cas.pak`: the tattoo pages list each category's stamps, every
    /// picture is in the pack, and every listed stamp has a decal texture and sane bounds.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn the_tattoo_categories_list_stamps_whose_pictures_and_decals_are_in_the_pack() {
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let names: std::collections::BTreeSet<&str> = index.iter().map(|e| e.0.as_str()).collect();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let mut builder = Builder::new(&read).unwrap();
        let data = builder.screen_data(&read).unwrap();
        // Every stamp of the pages has its decal in the pack, and a body wearing one builds with the decal on the legs.
        let decals = builder.library["tattoos"].as_object().unwrap();
        assert!(decals.len() >= 170);
        for (id, t) in decals {
            assert!(names.contains(t["texture"].as_str().unwrap()), "{id}");
        }
        let (stamp_id, _) = decals.iter().next().unwrap();
        let mut record = data.fallback[1].clone();
        record.graphics.push(skate_core::skater_record::Graphic { name: "t".into(), values: [0, u64::from_str_radix(stamp_id, 16).unwrap(), 0], flags: [0, 3] });
        record.adjust.words[2] = 0;
        // The lower tattoos are placed in the viewing pants, which show the legs.
        record.worn[Slot::Pants as usize] = data.catalog.tattoo_pants(Gender::Male).expect("viewing pants");
        let look = builder.assemble(&read, &record, false).unwrap();
        println!("{:?}", look.skater.materials.iter().map(|m| m.name.clone()).collect::<Vec<_>>());
        assert!(look.decals.textures[2].is_some() && look.decals.target.contains(&3), "{:?}", look.decals.target);
        assert_eq!(data.vault.tattoo_categories.len(), 3);
        assert_eq!(data.vault.stamp_camera("tattooupper"), Some("upper_tattoo"));
        assert_eq!(data.vault.stamp_camera("tattoolower"), Some("lower_tattoo"));
        let mut total = 0;
        for category in &data.vault.tattoo_categories {
            let rows = data.catalog.tattoo_rows(category);
            println!("category {}: {} stamps (untagged {})", category.text, rows.len(), category.untagged);
            assert!(rows.len() > 30, "{}", category.text);
            for item in rows {
                let key = skate_ui::fe::retail_menu_movies::image_key(&data.catalog.stamp_thumbnail(item));
                assert!(names.contains(format!("cas/thumbs/{key}.tex").as_str()), "{key}");
                let b = data.catalog.stamp_bounds(item);
                assert!(b[0] > b[1] && b[3] > b[2], "{b:?}");
                total += 1;
            }
        }
        println!("{total} stamps");
        // The Graphics pages: every brand and team logo has its picture and its decal, and a deck, a hat and a shirt
        // wearing one build with the decal on their piece.
        let logos = builder.library["logos"].as_object().unwrap();
        for (team, count) in [(false, 182), (true, 42)] {
            let rows = data.catalog.logo_rows(team);
            println!("logos team={team}: {}", rows.len());
            assert_eq!(rows.len(), count);
            for item in rows {
                let key = skate_ui::fe::retail_menu_movies::image_key(&data.catalog.stamp_thumbnail(item));
                assert!(names.contains(format!("cas/thumbs/{key}.tex").as_str()), "{key}");
                let entry = &logos[&format!("{:016x}", item.material)];
                assert!(names.contains(entry["texture"].as_str().unwrap()), "{item:?}");
            }
        }
        let logo = data.catalog.logo_rows(false)[0];
        let mut record = data.fallback[1].clone();
        for at in [0usize, 3, 4] {
            record.graphics.push(skate_core::skater_record::Graphic { name: "l".into(), values: [logo.model, logo.material, 0], flags: [0, at as u8 + 1] });
            record.adjust.words[at] = 0;
        }
        record.adjust.flag = 0;
        record.worn[Slot::Hat as usize] = data.catalog.shop_rows(Slot::Hat, Gender::Male, |_| true)[0];
        let look = builder.assemble(&read, &record, false).unwrap();
        println!("graphic targets {:?}", look.decals.target);
        for id in [1u8, 4, 5] {
            assert!(look.decals.target.contains(&id), "slot {id}: {:?}", look.decals.target);
        }
        // The arms' and legs' quarters come from their rows.
        let arm = data.fallback[1].worn[Slot::Arm as usize];
        let rect = data.catalog.stamp_rect(Some((Slot::Arm, arm)), 7);
        println!("arm quarter 7: {rect:?}");
        assert!(rect[0] > rect[1] && rect[3] > rect[2]);
    }

    /// Research: the screen's own flow to a placed tattoo, then the look built from its record.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn research_screen_flow_builds_a_tattoo() {
        use skate_core::cac::Cac;
        use skate_core::unlocks::UnlockStore;
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let mut builder = Builder::new(&read).unwrap();
        let data = std::sync::Arc::new(builder.screen_data(&read).unwrap());
        let mut c = Cac::open(data.fallback[1].clone(), &UnlockStore::fresh_profile(), Some(data.clone()));
        c.select(0); // Pete's
        let n = c.row_count() as i32;
        c.select(n - 2); // Upper Body Tattoo
        c.select(0); // front
        c.look_done();
        c.select(0); // category 1
        println!("menu {} rows {}", c.menu(), c.row_count());
        c.select_thumb(1);
        println!("menu {} graphics {:?} adjust {:?} flag {}", c.menu(), c.record.graphics, c.record.adjust.words, c.record.adjust.flag);
        let shown = c.shown_record().into_owned();
        let look = builder.assemble(&read, &shown, false).unwrap();
        println!("targets {:?} textures {:?}", look.decals.target, look.decals.textures.iter().map(Option::is_some).collect::<Vec<_>>());
        assert!(look.decals.textures[1].is_some(), "the first stamp of the first category has a decal");
        assert!(look.decals.target.contains(&2));
    }

    /// Research: where an upper tattoo at its start lands on the bare top's second UV set.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn research_upper_tattoo_coverage() {
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let mut builder = Builder::new(&read).unwrap();
        let data = builder.screen_data(&read).unwrap();
        let mut record = data.fallback[1].clone();
        let (id, _) = builder.library["tattoos"].as_object().unwrap().iter().next().map(|(k, v)| (k.clone(), v.clone())).unwrap();
        record.graphics.push(skate_core::skater_record::Graphic { name: "t".into(), values: [0, u64::from_str_radix(&id, 16).unwrap(), 0], flags: [0, 2] });
        record.adjust.words[1] = 0;
        record.adjust.flag = 1;
        let bare = data.catalog.bare_top(&record).unwrap();
        record.worn[Slot::OuterTorso as usize] = bare;
        let look = builder.assemble(&read, &record, false).unwrap();
        println!("targets {:?}", look.decals.target);
        let rows = skate_core::stamp::start(1, false).custom_graphic();
        for (m, mat) in look.skater.materials.iter().enumerate() {
            let mut n = 0;
            let mut inside = 0;
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in look.skater.primitives.iter().filter(|p| usize::from(p.material) == m) {
                for &i in &look.skater.indices[p.first_index as usize..(p.first_index + p.index_count) as usize] {
                    let uv = look.uv1[i as usize];
                    n += 1;
                    for k in 0..2 { lo[k] = lo[k].min(uv[k]); hi[k] = hi[k].max(uv[k]); }
                    let d = [uv[0] * rows[0][0] + uv[1] * rows[0][1] + rows[0][2], uv[0] * rows[1][0] + uv[1] * rows[1][1] + rows[1][2]];
                    if (0.0..=1.0).contains(&d[0]) && (0.0..=1.0).contains(&d[1]) { inside += 1; }
                }
            }
            println!("{} : {n} indices, uv1 {lo:?}..{hi:?}, inside {inside}", mat.name);
        }
    }

    /// `SKATEVITA` at the folder holding the real `cas.pak`: every deck colour has its deck, and a colourisable tee
    /// opens Wear Styles with the outer torso's swatches; its colour row names the tee in layer 9.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn wear_styles_colours_tint_the_item_and_every_deck_colour_has_a_deck() {
        use skate_core::cac::{Cac, Key, MENU_WEAR_STYLES};
        use skate_core::unlocks::UnlockStore;
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let data = std::sync::Arc::new(Builder::new(&read).unwrap().screen_data(&read).unwrap());
        let decks = data.vault.zone(11).expect("the deck zone");
        for colour in &decks.colours {
            assert!(data.catalog.deck_of_colour(&colour.tag).is_some(), "deck colour {}", colour.tag);
        }
        let mut store = UnlockStore::fresh_profile();
        store.unlock_all = true;
        let mut opened = Cac::open(data.fallback[1].clone(), &store, Some(data.clone()));
        opened.select(1);
        // The first tile of Slappy's tops pages that opens Wear Styles with a colour row.
        let mut found = None;
        'pages: for page in 1..5 {
            let mut c = opened.clone();
            c.select(page);
            for i in 0..c.row_count() as i32 {
                let mut t = c.clone();
                t.select_thumb(i);
                if t.menu() == MENU_WEAR_STYLES && t.row_kind(0) == "color" {
                    found = Some(t);
                    break 'pages;
                }
            }
        }
        let mut c = found.expect("a colourisable top");
        assert_eq!(c.row_kind(0), "color");
        assert_eq!(c.row_colours(0).len(), 3 * data.vault.zone(9).unwrap().colours.len());
        c.look_done();
        c.set_cursor(0, 0);
        c.key(Key::Right);
        let tee = c.record.worn[skate_core::skater_record::Slot::OuterTorso as usize];
        assert!(c.record.colours.iter().any(|l| l.index == 9 && l.part == tee));
    }

    /// `SKATEVITA` at the folder holding the real `cas.pak`: the screen's vault rows read as retail's.
    #[test]
    #[ignore = "requires a baked cas.pak (SKATEVITA)"]
    fn the_screen_data_reads_from_the_real_pack() {
        let dir = std::env::var("SKATEVITA").expect("SKATEVITA");
        let pack = std::fs::read(format!("{dir}/cas.pak")).unwrap();
        let index = vita_pack::files::index_of(&pack).unwrap();
        let read = |p: &str| {
            let (_, at, len) = index.iter().find(|e| e.0 == p).ok_or_else(|| format!("{p}: missing"))?;
            Ok(pack[*at as usize..*at as usize + len].to_vec())
        };
        let data = Builder::new(&read).unwrap().screen_data(&read).unwrap();
        let v = &data.vault;
        for z in &v.zones {
            println!("zone {:016x} slot {}: {:?}", z.key, z.slot, z.colours.iter().map(|c| c.tag.as_str()).collect::<Vec<_>>());
        }
        for p in &v.presets {
            println!("preset {} {} {} {}: {} morphs {:?}", p.key, p.male, p.number, p.label, p.morphs.len(), p.colours);
        }
        // Skin (2), hair (1), eyes (6) and facial hair (8) are the screen's colour rows.
        for slot in [1, 2, 6, 8] {
            assert!(v.zone(slot).is_some_and(|z| z.colours.len() > 1), "zone {slot}");
        }
        // The fallback male's skin is the first of its zone (skin_light_2 is order 1, after skin_light_3).
        let skin = v.zone(2).unwrap();
        assert!(skin.colours.iter().any(|c| c.key == data.fallback[1].colour_keys[2]));
        assert_eq!(v.morphs.len(), 19);
        assert_eq!(v.morph("nose_length").map(|m| (m.index, m.max)), Some((7, 0.5)));
        assert_eq!(v.presets.iter().filter(|p| p.male && p.number > 0).count(), 10);
        let preset = v.presets.iter().find(|p| p.key == "male_04").unwrap();
        assert!(preset.morphs.len() >= 17 && preset.colours.iter().all(|(z, _)| v.zones.iter().any(|zone| zone.key == *z)));
        let head = v.camera("head").unwrap();
        assert_eq!((head.eye, head.target, head.fov), ([-1.7, 1.42, -7.15], [3.0, 2.4, -7.5], 35.0));
    }
}
