//! Which ped a census spawn becomes: entity inside the category, its model (look), tints.
//!
//! Retail order (TU3): the census picks the category (`sub_826B8B88`, ported in
//! `census::pick_category`), then in the same function the entity: count = the category's
//! `entities` array (`Hash_D5E1267E2D715124`, `sub_8269B040`), draw `sub_826BB058`
//! (u32 x 2^-32 as f32), index = `trunc(draw x 100) % count` (unsigned remainder), count 0 = no
//! spawn [code]. The model's tints come from one rand `r` (`sub_827B4170`): `tints_a[r % na]`,
//! `tints_b[r % nb]`, a default vector when an array is empty [code]. `tints_a` is the model's
//! `secondary_colours` list (`Hash_DF76D7D773857EDB`, read first), `tints_b` its
//! `chassis_colours` (`Hash_12026E2EED18CC8D`) [code: the two hash keys in `sub_827B4170`]; the
//! ped shaders paint the atlas's red mask with `tints_a` and the blue mask with `tints_b`
//! (`colorize`).
//!
//! Our draws come from the spawn record's seed (one sub-RNG per ped, fixed draw order: entity,
//! group child, tints), not from retail's global RNG, so a ped's look is a pure function of its
//! spawn record. The stream differs from retail's; the distribution is the same.
//!
//! Group model records without a recipe (`skater_female`, `skater_male`, `worker_male` [data]):
//! the looks are the records that inherit from them. How retail picks among them is not read
//! from the code yet: placeholder = a uniform pick by the seed (`draw % children`), children in
//! key order. Parked in doc 26.

use crate::living_world::rng::{Rng, derive};
use std::collections::BTreeMap;

/// `livingworld_models` record (the fields M2 uses).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedModel {
    pub parent: Option<String>,
    /// `Hash_62F7C585686F3371` recipe name = GLB name (`models/<recipe>.glb`); empty = group record.
    pub recipe: String,
    /// `Hash_3EB8E0CD15F0891C` = the speech voice id (`world-ped-audio.md`).
    pub voice: Option<u32>,
    /// `secondary_colours`: the red mask tints (`i_colorize_red`).
    pub tints_a: Vec<[f32; 4]>,
    /// `chassis_colours`: the blue mask tints (`i_colorize_blue`).
    pub tints_b: Vec<[f32; 4]>,
    /// `Hash_73B6874C7B46C7C6` (45 / 55) and `Hash_9FCFDBEA56BA4733` (65 / 75) [data]: read by
    /// `sub_827C1188` [code]: the first is the camera distance fade (opaque to 45 m, gone at
    /// 55 m, `peds::fade`); the second replaces it only when its third float is larger.
    pub lod_near: Option<[f32; 2]>,
    pub lod_far: Option<[f32; 2]>,
}

/// `livingworld_entities` record (the fields M2 uses).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedEntity {
    /// `Model` ref (a `livingworld_models` key).
    pub model: Option<String>,
    /// `livingworld_entity_animation` set (`default`, `female`, `bum`, `jock`, ...).
    pub anim_set: Option<String>,
}

/// The tables the choice reads, keyed by retail record names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedCatalog {
    /// `livingworld_entitycategories`: category -> `entities` in data order.
    pub categories: BTreeMap<String, Vec<String>>,
    pub entities: BTreeMap<String, PedEntity>,
    pub models: BTreeMap<String, PedModel>,
}

/// Mod / engine overrides by key (empty = retail).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedOverrides {
    /// category -> replacement entity list.
    pub category_entities: BTreeMap<String, Vec<String>>,
    /// entity -> model record.
    pub entity_model: BTreeMap<String, String>,
    /// entity -> animation set.
    pub entity_anim_set: BTreeMap<String, String>,
    /// model record -> replacement tint palettes `(tints_a, tints_b)` (red mask, blue mask);
    /// picked with the same one-rand rule as the retail lists.
    pub model_tints: BTreeMap<String, (Vec<[f32; 4]>, Vec<[f32; 4]>)>,
}

/// The resolved look of one ped.
#[derive(Clone, Debug, PartialEq)]
pub struct PedLook {
    pub entity: String,
    /// The model record that carries the recipe (a group record's chosen child).
    pub model: String,
    pub recipe: String,
    pub anim_set: String,
    pub voice: Option<u32>,
    pub tint_a: [f32; 4],
    pub tint_b: [f32; 4],
}

/// Seed label of the look sub-RNG.
pub const LOOK_LABEL: u64 = 0x5045_444C_4F4F_4B00;
/// The default tint when a model has no tint array (retail reads a constant vector at
/// `0x830D0850`; its value is runtime data, not read: white = no tint).
pub const DEFAULT_TINT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// `sub_826BB058`: u32 x 2^-32 rounded to f32 (`fcfid`, `frsp`, `fmuls`). Can reach 1.0.
pub fn unit_f32(draw: u32) -> f32 {
    (draw as f32) * f32::from_bits(0x2F80_0000)
}

/// `sub_826B8B88`: the entity index for one draw, `None` for an empty list.
pub fn entity_index(draw: u32, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let scaled = unit_f32(draw) * 100.0;
    let n = scaled as i32 as u32;
    Some((n % count as u32) as usize)
}

/// `sub_827B4170`: both tints from one rand.
pub fn tints(model: &PedModel, r: u32) -> ([f32; 4], [f32; 4]) {
    let pick = |v: &[[f32; 4]]| if v.is_empty() { DEFAULT_TINT } else { v[(r % v.len() as u32) as usize] };
    (pick(&model.tints_a), pick(&model.tints_b))
}

impl PedCatalog {
    /// The records inheriting from a group model record, with a recipe, in key order.
    pub fn model_children(&self, model: &str) -> Vec<&str> {
        self.models.iter().filter(|(_, m)| m.parent.as_deref() == Some(model) && !m.recipe.is_empty()).map(|(k, _)| k.as_str()).collect()
    }

    /// The look of a census spawn: category + the spawn record's seed. `None` when the category
    /// has no entities or the entity has no usable model (retail: no spawn).
    pub fn choose(&self, category: &str, seed: u64, overrides: &PedOverrides) -> Option<PedLook> {
        let mut rng = Rng::new(derive(seed, &[LOOK_LABEL]));
        let draws = [rng.next_u32(), rng.next_u32(), rng.next_u32()];
        let list = overrides.category_entities.get(category).or_else(|| self.categories.get(category))?;
        let entity = list.get(entity_index(draws[0], list.len())?)?.clone();
        self.look_for(&entity, draws[1], draws[2], overrides)
    }

    /// The look of a given entity (placed spawns, mods): draws for the group child and tints.
    pub fn look_for(&self, entity: &str, child_draw: u32, tint_draw: u32, overrides: &PedOverrides) -> Option<PedLook> {
        let record = self.entities.get(entity);
        let model_key = overrides.entity_model.get(entity).cloned().or_else(|| record.and_then(|e| e.model.clone()))?;
        let mut model_key = model_key;
        let mut model = self.models.get(&model_key)?;
        if model.recipe.is_empty() {
            let children = self.model_children(&model_key);
            if children.is_empty() {
                return None;
            }
            model_key = children[(child_draw % children.len() as u32) as usize].to_string();
            model = &self.models[&model_key];
        }
        let (tint_a, tint_b) = match overrides.model_tints.get(&model_key) {
            Some((a, b)) => tints(&PedModel { tints_a: a.clone(), tints_b: b.clone(), ..PedModel::default() }, tint_draw),
            None => tints(model, tint_draw),
        };
        let anim_set = overrides.entity_anim_set.get(entity).cloned().or_else(|| record.and_then(|e| e.anim_set.clone())).unwrap_or_else(|| "default".into());
        Some(PedLook { entity: entity.to_string(), model: model_key, recipe: model.recipe.clone(), anim_set, voice: model.voice, tint_a, tint_b })
    }
}
