//! Pedestrian body data (doc 26, peds milestone M2): the ped animation bank and the look /
//! animation tables, read into the pure `skate_core::living_world::peds` types.
//!
//! - Bank: `data/anim/PedestrianSkeletonPres.abin` from the user's stock data (462 VBR clips, one
//!   50-bone rig with trajectory, 10 parts) [data]. Its clips carry only the first 6 parts (bones
//!   0..=26), so the skater's full-hierarchy loader (`animation_frames`) does not apply; this
//!   module decodes the parts a clip has with the same core VBR decoder and leaves the other
//!   bones at identity. The reference pose is the bank's `PEDESTRIAN_RIG_TPOSE` pose record.
//!   Clip attributes (`LEFTTOEDOWN`, `RIGHTTOEDOWN`, `LEFTHEELDOWN`, `RIGHTHEELDOWN`,
//!   `BODYFALLTYPE`, ...) come from `animation_metadata` (first payload word = the value).
//! - Tables (`private/living_world/tables.json`): `livingworld_entitycategories.entities`,
//!   `livingworld_entities` (`model`, the `livingworld_entity_animation` ref), `livingworld_models`
//!   (`recipe`, `voice`, `tints_a` / `tints_b` = `secondary_colours` / `chassis_colours`, distance pairs) and the animation sets, whose
//!   logical names are vault hashes (`attrib_hash::hash("FwdWalkCyc")` = `Hash_4AA12E0083F10739`)
//!   and whose `tAnimAttributes` entries name their clip through `anim_name` (setup resolves the
//!   string-pool offset, `living_world_anim.py`).

use crate::abin::{Bank, RecordData};
use crate::animation_metadata::AnimationMetadata;
use serde_json::Value;
use skate_core::animation::output::Sqt;
use skate_core::animation::vbr::VbrDecoder;
use skate_core::living_world::peds::anim::{ClipWindow, IDENTITY, PedAnimSet, PedClip, PedRig, RemapClip, names};
use skate_core::living_world::peds::{PedCatalog, PedEntity, PedModel};
use std::collections::BTreeMap;
use std::path::Path;

/// The bank under an asset root.
pub const PED_BANK: &str = "private/stock/data/anim/PedestrianSkeletonPres.abin";
/// The bank's reference pose record.
pub const REFERENCE_POSE: &str = "PEDESTRIAN_RIG_TPOSE";

fn sqt(words: [u32; 10]) -> Sqt {
    let [sx, sy, sz, x, y, z, w, tx, ty, tz] = words.map(f32::from_bits);
    Sqt { scale: [sx, sy, sz, 1.0], rotation: [x, y, z, w], translation: [tx, ty, tz, 1.0] }
}

/// The ped animation bank: rig, reference pose, clip decoding.
pub struct PedBank {
    pub bank: Bank,
    pub metadata: AnimationMetadata,
    pub rig: PedRig,
}

impl PedBank {
    pub fn load(asset_root: &Path) -> Result<Self, String> {
        let path = asset_root.join(PED_BANK);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(bytes)
    }

    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        let bank = Bank::parse(bytes).map_err(|e| e.to_string())?;
        let metadata = AnimationMetadata::from_bank(&bank, "PedestrianSkeletonPres.abin".into(), "0".repeat(64))?;
        let h = bank.hierarchy().ok_or("ped bank has no hierarchy")?.clone();
        let n = h.bone_count as usize;
        let mut rig = PedRig { names: h.bone_names.clone(), parents: h.parents.clone(), mirrors: h.mirrors.clone(), reference: vec![IDENTITY; n], animated: vec![false; n] };
        let mut this = Self { bank, metadata, rig: PedRig::default() };
        if let Some((_, pose)) = this.bank.pose(REFERENCE_POSE) {
            let parts = pose.parts.clone();
            let (frames, present) = this.decode_parts(&parts, 1)?;
            for (i, p) in present.iter().enumerate() {
                if *p {
                    rig.reference[i] = frames[0][i];
                }
            }
        } else {
            return Err(format!("ped bank has no {REFERENCE_POSE} pose"));
        }
        // Bones every clip carries: the parts all clips have.
        let parts = this.bank.records().iter().filter_map(|r| if let RecordData::Clip(c) = &r.data { Some(c.parts.len()) } else { None }).min().unwrap_or(0);
        for layout in h.parts.iter().take(parts) {
            for b in layout.sqt_offset as usize..(layout.sqt_offset as usize + layout.bone_count as usize).min(n) {
                rig.animated[b] = true;
            }
        }
        this.rig = rig;
        Ok(this)
    }

    /// A channel clip's per-bone weights (each part's table of one big-endian f32 per channel, bones of parts without a
    /// table weigh 0); `None` when no part has a table.
    fn channel_weights(&self, parts: &[crate::abin::PartEntry]) -> Option<Vec<f32>> {
        let h = self.bank.hierarchy()?;
        let mut weights = vec![0.0; h.bone_count as usize];
        let mut any = false;
        for (entry, layout) in parts.iter().zip(&h.parts) {
            let Some(range) = entry.part.as_ref().and_then(|p| p.channel_weights.clone()) else { continue };
            let bytes = self.bank.bytes().get(range)?;
            for (i, w) in bytes.chunks_exact(4).enumerate() {
                if let Some(slot) = weights.get_mut(layout.sqt_offset as usize + i) {
                    *slot = f32::from_be_bytes([w[0], w[1], w[2], w[3]]);
                    any = true;
                }
            }
        }
        any.then_some(weights)
    }

    /// Decode the parts present (clip part i = hierarchy part i); absent bones stay identity.
    fn decode_parts(&self, parts: &[crate::abin::PartEntry], frames: usize) -> Result<(Vec<Vec<Sqt>>, Vec<bool>), String> {
        let h = self.bank.hierarchy().ok_or("no hierarchy")?;
        let n = h.bone_count as usize;
        let mut out = vec![vec![IDENTITY; n]; frames];
        let mut present = vec![false; n];
        for (entry, layout) in parts.iter().zip(&h.parts) {
            let Some(part) = &entry.part else { continue };
            let mut d = VbrDecoder::new(self.bank.bytes(), part.offset, part.compression_header_relative as usize, part.compressed_data_relative as usize)?;
            if d.frame_count() != frames || d.channel_count() != layout.bone_count as usize {
                return Err(format!("{}: part frame / channel count differs from the header", layout.name));
            }
            let start = layout.sqt_offset as usize;
            for (f, frame) in out.iter_mut().enumerate() {
                for (i, w) in d.decode_frame(f)?.into_iter().enumerate() {
                    if let Some(slot) = frame.get_mut(start + i) {
                        *slot = sqt(w);
                    }
                }
            }
            for p in present.iter_mut().skip(start).take(layout.bone_count as usize) {
                *p = true;
            }
        }
        Ok((out, present))
    }

    pub fn clip_names(&self) -> Vec<String> {
        self.bank.records().iter().filter(|r| matches!(r.data, RecordData::Clip(_))).map(|r| r.header.name.clone()).collect()
    }

    /// Decode one clip with its attribute windows.
    pub fn clip(&self, name: &str) -> Result<PedClip, String> {
        let (header, clip) = self.bank.clip(name).ok_or_else(|| format!("ped clip {name} not in the bank"))?;
        let frames = f32::from_bits(clip.frame_count_bits);
        if !frames.is_finite() || frames < 1.0 || frames.fract() != 0.0 {
            return Err(format!("{name}: invalid frame count"));
        }
        let (frames, _) = self.decode_parts(&clip.parts, frames as usize).map_err(|e| format!("{name}: {e}"))?;
        let meta = self.metadata.clip(&header.name)?;
        let windows = meta
            .attributes
            .iter()
            .map(|a| ClipWindow {
                channel: a.name.clone(),
                begin: f32::from_bits(a.begin_bits),
                end: f32::from_bits(a.end_bits),
                value: a.payload_words.first().map_or(0.0, |w| f32::from_bits(*w)),
            })
            .collect();
        let speed = f32::from_bits(clip.base_speed_bits);
        Ok(PedClip { channel_weights: self.channel_weights(&clip.parts),
            name: header.name.clone(),
            fps: f32::from_bits(clip.fps_bits) * if speed.is_finite() && speed > 0.0 { speed } else { 1.0 },
            frames,
            looping: clip.looping(),
            loop_rotation: clip.loop_rotation_words.map(f32::from_bits),
            loop_translation: [f32::from_bits(clip.loop_translation_words[0]), f32::from_bits(clip.loop_translation_words[1]), f32::from_bits(clip.loop_translation_words[2])],
            windows,
        })
    }
}

/// The look and animation tables M2 reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PedTables {
    pub catalog: PedCatalog,
    /// `livingworld_entity_animation` sets by record name.
    pub anim_sets: BTreeMap<String, PedAnimSet>,
    /// Remap entries that name a clip only by a string-pool offset (tables exported before the
    /// clip-name resolver): re-run setup to fill them.
    pub unresolved: usize,
}

fn f32s(v: &Value) -> Option<[f32; 4]> {
    Some([v["x"].as_f64()? as f32, v["y"].as_f64()? as f32, v["z"].as_f64()? as f32, v["w"].as_f64().unwrap_or(1.0) as f32])
}

fn pair(v: &Value) -> Option<[f32; 2]> {
    Some([v["f32_0"].as_f64()? as f32, v["f32_4"].as_f64()? as f32])
}

/// A `recovery_intervals*` struct (`lying_down_until`, `crouched_until`) by its export name or raw hash key.
fn intervals(fields: &Value, name: &str, hash: &str) -> Option<[f32; 2]> {
    let v = fields.get(name).or_else(|| fields.get(hash))?;
    Some([v.get("lying_down_until")?.as_f64()? as f32, v.get("crouched_until")?.as_f64()? as f32])
}

/// One remap value: a clip name, a list of names, a `tAnimAttributes` struct or a list of them.
fn remap(v: &Value, unresolved: &mut usize) -> Vec<RemapClip> {
    match v {
        Value::String(s) if !s.is_empty() && !s.contains('/') => vec![RemapClip { clip: s.clone(), windows: vec![] }],
        Value::Array(items) => items.iter().flat_map(|i| remap(i, unresolved)).collect(),
        Value::Object(o) if o.contains_key("anim") => match o.get("anim_name").and_then(Value::as_str) {
            Some(name) => {
                let windows = ["window_0", "window_1", "window_2"]
                    .iter()
                    .filter_map(|k| {
                        let w = o.get(*k)?.as_array()?;
                        let tag = w.get(2)?.as_i64()? as i32;
                        (tag != 0).then_some((w.first()?.as_f64()? as f32, w.get(1)?.as_f64()? as f32, tag))
                    })
                    .collect();
                vec![RemapClip { clip: name.to_string(), windows }]
            }
            None => {
                *unresolved += 1;
                vec![]
            }
        },
        _ => vec![],
    }
}

impl PedTables {
    pub fn parse(json: &[u8]) -> Result<Self, String> {
        let doc: Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        Self::from_value(&doc)
    }

    pub fn from_value(doc: &Value) -> Result<Self, String> {
        let classes = doc.get("classes").ok_or("tables.json has no classes")?;
        let class = |n: &str| classes.get(n).and_then(Value::as_object);
        let mut t = PedTables::default();
        for (k, row) in class("livingworld_entitycategories").into_iter().flatten() {
            let list = row["fields"]["entities"].as_array().map(|a| a.iter().filter_map(|r| r["key"].as_str().map(String::from)).collect()).unwrap_or_default();
            t.catalog.categories.insert(k.clone(), list);
        }
        for (k, row) in class("livingworld_entities").into_iter().flatten() {
            let fields = row["fields"].as_object();
            let model = row["fields"]["model"]["key"].as_str().map(String::from);
            // The animation set is the one field referencing that class (its key is a hash).
            let anim_set = fields.and_then(|f| f.values().find(|v| v["class"] == "livingworld_entity_animation")).and_then(|v| v["key"].as_str()).map(String::from);
            t.catalog.entities.insert(k.clone(), PedEntity { model, anim_set });
        }
        for (k, row) in class("livingworld_models").into_iter().flatten() {
            let f = &row["fields"];
            // `sub_827B4170` reads `secondary_colours` (red mask) then `chassis_colours` (blue
            // mask) [code]; the export names them, the raw hash key is accepted too.
            let tints = |name: &str, hash: &str| f.get(name).or_else(|| f.get(hash)).and_then(Value::as_array).map(|a| a.iter().filter_map(f32s).collect()).unwrap_or_default();
            t.catalog.models.insert(
                k.clone(),
                PedModel {
                    parent: row["parent"].as_str().map(String::from),
                    recipe: f["recipe"].as_str().unwrap_or("").to_string(),
                    voice: f["voice"].as_u64().map(|v| v as u32),
                    tints_a: tints("secondary_colours", "Hash_DF76D7D773857EDB"),
                    tints_b: tints("chassis_colours", "Hash_12026E2EED18CC8D"),
                    lod_near: pair(&f["Hash_73B6874C7B46C7C6"]),
                    lod_far: pair(&f["Hash_9FCFDBEA56BA4733"]),
                },
            );
        }
        let keys: Vec<(String, String)> = names::ALL.iter().map(|n| (crate::attrib_hash::numeric_name(n), n.to_string())).collect();
        for (k, row) in class("livingworld_entity_animation").into_iter().flatten() {
            let mut set = PedAnimSet::default();
            for (hash, name) in &keys {
                if let Some(v) = row["fields"].get(hash) {
                    let list = remap(v, &mut t.unresolved);
                    if !list.is_empty() {
                        set.entries.insert(name.clone(), list);
                    }
                }
            }
            // Collision values (`sub_82E38FB8`, `sub_8269A990`) [code + data]; the export names the
            // speeds, the raw hash keys are accepted too; a missing value keeps the stock default.
            let f = &row["fields"];
            let num = |name: &str, hash: &str| f.get(name).or_else(|| f.get(hash)).and_then(Value::as_f64).map(|v| v as f32);
            let d = set.collision;
            set.collision = skate_core::living_world::peds::skater_contact::CollisionRules {
                knockdown_speed: [
                    num("knockdown_speed_a", "Hash_541FFA2E9D81C947").unwrap_or(d.knockdown_speed[0]),
                    num("knockdown_speed_b", "Hash_7C5E39ECE5A5572E").unwrap_or(d.knockdown_speed[1]),
                ],
                ped_knockdown_speed: [
                    num("knockdown_speed_flagged_a", "Hash_081F978F8706D77B").unwrap_or(d.ped_knockdown_speed[0]),
                    num("knockdown_speed_flagged_b", "Hash_5E566360FDC5FDD1").unwrap_or(d.ped_knockdown_speed[1]),
                ],
                can_knock_down: f.get("Hash_5B92564B352A9FAA").and_then(Value::as_bool).unwrap_or(d.can_knock_down),
                ground_seconds: num("ground_seconds", "Hash_AD3C483F0C9DAD67").unwrap_or(d.ground_seconds),
                // `sub_8269E460`'s interval pairs (b372 / b373): named structs in the export, raw hashes accepted.
                recovery_front: intervals(f, "recovery_intervals_b", "Hash_E37F00409DEAB4D2").unwrap_or(d.recovery_front),
                recovery_back: intervals(f, "recovery_intervals", "Hash_74C0B72217A7704E").unwrap_or(d.recovery_back),
                recovery_side: intervals(f, "recovery_intervals_left", "Hash_0162B7A9B0C086EE").unwrap_or(d.recovery_side),
                // `Wipeout{Back,Fwd,Left}GroundRestartTime` (knockdown.xml frame offset tags), keyed by the logical
                // name's hash.
                ground_restart: ["WipeoutBackGroundRestartTime", "WipeoutFwdGroundRestartTime", "WipeoutLeftGroundRestartTime"]
                    .map(|n| crate::attrib_hash::numeric_name(n))
                    .iter()
                    .zip(d.ground_restart)
                    .map(|(k, dv)| f.get(k.as_str()).and_then(Value::as_f64).map_or(dv, |v| v as f32))
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap_or(d.ground_restart),
                // Ours (retail's `c+4 > 5` is a code constant): a mod may set it per set.
                block_after: f.get("block_after").and_then(Value::as_u64).map_or(d.block_after, |v| v.min(255) as u8),
            };
            t.anim_sets.insert(k.clone(), set);
        }
        if t.catalog.categories.is_empty() || t.catalog.models.is_empty() {
            return Err("tables.json has no ped categories or models".into());
        }
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Data-gated: the carry clips are channel clips whose per-bone weights pick the bones they take over (the rest
    /// weigh 0); walk cycles carry no weight table.
    #[test]
    fn carry_clips_carry_per_bone_channel_weights() {
        let Some(root) = std::env::var_os("SKATE3_ASSET_ROOT").map(std::path::PathBuf::from).filter(|r| r.join(PED_BANK).exists()) else {
            eprintln!("skipped: set SKATE3_ASSET_ROOT to the converted assets");
            return;
        };
        let bank = PedBank::load(&root).expect("ped bank");
        let h = bank.bank.hierarchy().unwrap();
        for name in ["NPC_CARRY_SML_RH_0_CYC", "NPC_CARRY_BIG_RH_0_CYC", "NPC_CARRY_PAPER_RH_0_CYC", "NPC_CARRY_WINE_RH_0_CYC"] {
            let clip = bank.clip(name).unwrap();
            let w = clip.channel_weights.unwrap_or_else(|| panic!("{name}: no channel weights"));
            let on: Vec<String> = w.iter().enumerate().filter(|(_, v)| **v > 0.0).map(|(i, v)| format!("{}={v}", h.bone_names.get(i).cloned().unwrap_or_default())).collect();
            eprintln!("{name}: {} of {} bones weighted: {}", on.len(), w.len(), on.join(" "));
            assert!(!on.is_empty() && on.len() < w.len());
        }
        assert!(bank.clip("NPC_WNDR_WLK_N_0_CYC").unwrap().channel_weights.is_none());
        for name in ["NPC_THROW_LIGHTRH_N_0_N", "NPC_ATCK_THROWRH_N_0_N", "NPC_ATCK_THROWRH_L_90_N", "SG_TAZR_TAZR_F_0_CYC"] {
            let w = bank.clip(name).unwrap().channel_weights;
            eprintln!("{name}: {:?} weighted bones", w.map(|w| w.iter().filter(|v| **v > 0.0).count()));
        }
    }

    #[test]
    fn logical_names_hash_like_the_export() {
        // Field keys seen in the exported tables [data].
        assert_eq!(crate::attrib_hash::numeric_name("FwdWalkCyc"), "Hash_4AA12E0083F10739");
        assert_eq!(crate::attrib_hash::numeric_name("IdleBasicCyc"), "Hash_ECC7EF02769736DB");
        assert_eq!(crate::attrib_hash::numeric_name("Stand2Walk"), "Hash_8A7544FD90ECFFF7");
    }

    #[test]
    fn tables_resolve_categories_models_and_remaps() {
        let doc = serde_json::json!({"classes": {
            "livingworld_entitycategories": {"aletown": {"parent": null, "fields": {"entities": [{"class": "livingworld_entities", "key": "jock02"}]}}},
            "livingworld_entities": {"jock02": {"parent": "jock", "fields": {"model": {"class": "livingworld_models", "key": "jock02"},
                "Hash_00367B8F33E79C43": {"class": "livingworld_entity_animation", "key": "jock"}}}},
            "livingworld_models": {"jock02": {"parent": "jock", "fields": {"recipe": "male_jock_2", "voice": 55,
                "secondary_colours": [{"x": 1.0, "y": 0.5, "z": 0.25, "w": 1.0}], "Hash_12026E2EED18CC8D": [{"x": 0.0, "y": 0.0, "z": 1.0, "w": 1.0}, {"x": 0.5, "y": 0.5, "z": 0.5, "w": 1.0}],
                "Hash_73B6874C7B46C7C6": {"f32_0": 45.0, "f32_4": 55.0, "f32_8": 0.0}}}},
            "livingworld_entity_animation": {"jock": {"parent": null, "fields": {
                "Hash_4AA12E0083F10739": {"anim": 52075, "anim_name": "NPC_WNDR_WLK_N_0_CYC", "window_0": [0.0, 0.051, 1], "window_1": [0.4166, 0.584, -1], "window_2": [0.0, 0.0, 0]},
                "Hash_ECC7EF02769736DB": [{"anim": 1, "anim_name": "IDLE_A"}, {"anim": 2}],
                "Hash_8A7544FD90ECFFF7": "NPC_WNDR_STND2WLK_N_0_N"}}}}});
        let t = PedTables::from_value(&doc).unwrap();
        assert_eq!(t.catalog.categories["aletown"], vec!["jock02".to_string()]);
        assert_eq!(t.catalog.entities["jock02"].anim_set.as_deref(), Some("jock"));
        let m = &t.catalog.models["jock02"];
        assert_eq!((m.recipe.as_str(), m.voice, m.tints_a.len(), m.lod_near), ("male_jock_2", Some(55), 1, Some([45.0, 55.0])));
        assert_eq!((m.tints_a[0], m.tints_b.len()), ([1.0, 0.5, 0.25, 1.0], 2));
        let set = &t.anim_sets["jock"];
        assert_eq!(set.entries[names::WALK][0].clip, "NPC_WNDR_WLK_N_0_CYC");
        assert_eq!(set.entries[names::WALK][0].windows, vec![(0.0, 0.051, 1), (0.4166, 0.584, -1)]);
        assert_eq!(set.entries[names::IDLE].len(), 1);
        assert_eq!(set.entries[names::START][0].clip, "NPC_WNDR_STND2WLK_N_0_N");
        assert_eq!(t.unresolved, 1);
    }
}
