//! Data-gated checks of the ped body data (doc 26, peds M2) on the user's own files. Skips (passes
//! with a note) without data. `SKATE3_ASSET_ROOT`: one or more asset roots joined like PATH; the
//! test reads `private/stock/data/anim/PedestrianSkeletonPres.abin` and
//! `private/living_world/{tables.json, models/*.glb}` from the first root that has each.
//! Expected values are shipped data the research measured; they are asserted, not embedded.

use skate_core::living_world::peds::anim::{PedAnimPlayer, names, root_motion};
use skate_core::living_world::peds::{PedOverrides, match_bones};
use skate_data::ped_anim::{PED_BANK, PedBank, PedTables};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn roots() -> Vec<PathBuf> {
    std::env::var_os("SKATE3_ASSET_ROOT").map(|r| std::env::split_paths(&r).collect()).unwrap_or_default()
}

fn find(rel: &str) -> Option<PathBuf> {
    roots().into_iter().map(|r| r.join(rel)).find(|p| p.exists())
}

fn bank() -> Option<PedBank> {
    let path = find(PED_BANK)?;
    let root = path.ancestors().nth(5)?.to_path_buf();
    Some(PedBank::load(&root).unwrap())
}

fn tables() -> Option<PedTables> {
    Some(PedTables::parse(&std::fs::read(find("private/living_world/tables.json")?).unwrap()).unwrap())
}

/// Minimal GLB reader: skin joint names and inverse bind matrices (column-major).
fn glb_skin(path: &std::path::Path) -> (Vec<String>, Vec<[f32; 16]>) {
    let b = std::fs::read(path).unwrap();
    let json_len = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
    let j: serde_json::Value = serde_json::from_slice(&b[20..20 + json_len]).unwrap();
    let bin_at = 20 + json_len + 8;
    let skin = &j["skins"][0];
    let names = skin["joints"].as_array().unwrap().iter().map(|i| j["nodes"][i.as_u64().unwrap() as usize]["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let acc = &j["accessors"][skin["inverseBindMatrices"].as_u64().unwrap() as usize];
    let view = &j["bufferViews"][acc["bufferView"].as_u64().unwrap() as usize];
    let start = bin_at + view["byteOffset"].as_u64().unwrap_or(0) as usize + acc["byteOffset"].as_u64().unwrap_or(0) as usize;
    let ibm = (0..acc["count"].as_u64().unwrap() as usize)
        .map(|k| std::array::from_fn(|i| f32::from_le_bytes(b[start + 64 * k + 4 * i..start + 64 * k + 4 * i + 4].try_into().unwrap())))
        .collect();
    (names, ibm)
}

/// Translation of the inverse of an affine column-major matrix.
fn inverse_translation(m: &[f32; 16]) -> [f32; 3] {
    // m = [R t; 0 1] -> inverse translation = -R^T t.
    let r = |row: usize, col: usize| m[col * 4 + row];
    let t = [m[12], m[13], m[14]];
    std::array::from_fn(|i| -(r(0, i) * t[0] + r(1, i) * t[1] + r(2, i) * t[2]))
}

#[test]
fn ped_bank_rig_reference_and_clips() {
    let Some(bank) = bank() else {
        eprintln!("skipped: set SKATE3_ASSET_ROOT to a root with {PED_BANK}");
        return;
    };
    let rig = &bank.rig;
    assert_eq!(rig.names.len(), 50, "50-bone rig [data]");
    assert_eq!(rig.names[0], "TRAJECTORY");
    assert_eq!(rig.animated.iter().filter(|a| **a).count(), 27, "clips carry bones 0..=26 (6 of 10 parts)");
    assert!((rig.reference[1].translation[1] - 0.921).abs() < 0.01, "hips height {:?}", rig.reference[1].translation);
    let names = bank.clip_names();
    assert_eq!(names.len(), 462);
    let mut channels = BTreeMap::new();
    for n in &names {
        let c = bank.clip(n).unwrap_or_else(|e| panic!("{e}"));
        assert!(c.frames.iter().flatten().all(|s| s.rotation.iter().chain(&s.translation).all(|v| v.is_finite())), "{n}");
        for w in &c.windows {
            *channels.entry(w.channel.clone()).or_insert(0) += 1;
        }
    }
    eprintln!("clip channels: {channels:?}");
    assert!(channels.contains_key("LEFTTOEDOWN") && channels.contains_key("RIGHTTOEDOWN") && channels.contains_key("BODYFALLTYPE"));
    let walk = bank.clip("NPC_WNDR_WLK_N_0_CYC").unwrap();
    assert!(walk.looping);
    let m = root_motion(&walk, 0.0, walk.length(), false, false);
    let speed = m.translation[2] / walk.length();
    eprintln!("NPC walk: {:.3} s, root speed {speed:.3} m/s, loop {:?}", walk.length(), walk.loop_translation);
    // Retail walking median 1.30 m/s (recomp PEDXYZ, peds-re 3); the clip's own trajectory.
    assert!((1.2..1.4).contains(&speed), "{speed}");
    let turn = bank.clip("NPC_WNDR_STND_R_180_N").unwrap();
    let yaw = root_motion(&turn, 0.0, turn.length(), false, false).yaw;
    eprintln!("NPC stand turn 180: yaw {:.3} rad", yaw);
    assert!((yaw.abs() - std::f32::consts::PI).abs() < 0.2, "{yaw}");
}

#[test]
fn every_census_entity_resolves_a_look_and_animation() {
    let (Some(t), Some(bank)) = (tables(), bank()) else {
        eprintln!("skipped: needs tables.json (with clip names) and the ped bank under SKATE3_ASSET_ROOT");
        return;
    };
    let clips: std::collections::BTreeSet<String> = bank.clip_names().into_iter().collect();
    // Census record -> group -> categories (the population's roll) -> entities.
    let census = skate_data::living_world::LivingWorldTables::parse(&std::fs::read(find("private/living_world/tables.json").unwrap()).unwrap()).unwrap();
    let used: std::collections::BTreeSet<&String> = ["aletown", "business_center", "mall", "memorial", "residential", "campus", "downtown", "university", "industrial", "reclaimed", "loadingdocks", "observatory"]
        .iter()
        .flat_map(|r| census.census[*r].categories.iter())
        .filter_map(|c| t.catalog.categories.get(&c.name))
        .flatten()
        .collect();
    eprintln!("unresolved remap entries: {}", t.unresolved);
    // Collision values [data]: knock-downs above 3.0, allowed except for the marquee sets, 1.5 s on
    // the ground (security 0.5 s); the reaction animations are named in every regular set.
    let c = |k: &str| t.anim_sets[k].collision;
    assert_eq!((c("default").knockdown_speed, c("default").can_knock_down, c("default").ground_seconds), ([3.0, 3.0], true, 1.5));
    assert!(!c("marquee").can_knock_down && c("security01").ground_seconds == 0.5);
    for name in skate_core::living_world::peds::skater_contact::REACTION_ANIMS {
        assert!(t.anim_sets["default"].entries.contains_key(*name), "default set: {name} missing");
    }
    // The idle hand prop actions (`82E3F1D8` / `82E3F470`) resolve in the default set too.
    for name in ["HandPropEat", "HandPropDrink", "HandPropReadInto", "HandPropReadCyc", "HandPropReadOut", "HandPropPhoneAnswer", "HandPropPhoneTalk", "HandPropPhoneHangup", "HandPropLitter"] {
        assert!(t.anim_sets["default"].entries.contains_key(name), "default set: {name} missing");
    }
    let o = PedOverrides::default();
    let mut recipes = std::collections::BTreeSet::new();
    for entity in &used {
        for draw in 0..8u32 {
            let look = t.catalog.look_for(entity, draw, draw, &o).unwrap_or_else(|| panic!("{entity}: no look"));
            if let Some(glb) = find(&format!("private/living_world/models/{}.glb", look.recipe)) {
                assert!(glb.exists());
            }
            recipes.insert(look.recipe.clone());
            let set = t.anim_sets.get(&look.anim_set).unwrap_or_else(|| panic!("{entity}: set {}", look.anim_set));
            for name in [names::IDLE, names::WALK, names::START, names::STOP, names::TURN_180] {
                let list = set.entries.get(name).unwrap_or_else(|| panic!("{}: {name} missing (re-run setup for clip names?)", look.anim_set));
                for c in list {
                    assert!(clips.contains(&c.clip), "{}: {name} -> {} not in the bank", look.anim_set, c.clip);
                }
            }
            assert!(PedAnimPlayer::new(set, draw as u64).is_some());
            // Collision reactions: every reaction animation resolves to clips in the bank.
            for name in skate_core::living_world::peds::skater_contact::REACTION_ANIMS {
                for c in set.entries.get(*name).into_iter().flatten() {
                    assert!(clips.contains(&c.clip), "{}: {name} -> {} not in the bank", look.anim_set, c.clip);
                }
            }
        }
    }
    eprintln!("{} census entities, {} recipes", used.len(), recipes.len());
}

#[test]
fn ped_tints_are_the_model_palettes_and_the_base_pair_is_the_identity() {
    let Some(t) = tables() else {
        eprintln!("skipped: needs tables.json under SKATE3_ASSET_ROOT");
        return;
    };
    // The root record's pair turns the ped shader rule into the identity only as
    // secondary -> red mask (tints_a), chassis -> blue mask (tints_b) [data].
    let root = &t.catalog.models["default"];
    assert_eq!((root.tints_a.as_slice(), root.tints_b.as_slice()), ([[1.0, 0.0, 0.0, 1.0]].as_slice(), [[0.0, 0.0, 1.0, 1.0]].as_slice()));
    let o = PedOverrides::default();
    let mut checked = 0;
    for (entity, record) in &t.catalog.entities {
        let Some(model) = record.model.as_ref().and_then(|m| t.catalog.models.get(m)) else { continue };
        if model.recipe.is_empty() || model.recipe.starts_with("zprop") {
            continue;
        }
        for draw in 0..20u32 {
            let look = t.catalog.look_for(entity, draw, draw, &o).unwrap();
            let m = &t.catalog.models[&look.model];
            assert!(!m.tints_a.is_empty() && !m.tints_b.is_empty(), "{}: empty palette", look.model);
            assert_eq!(look.tint_a, m.tints_a[draw as usize % m.tints_a.len()], "{entity}");
            assert_eq!(look.tint_b, m.tints_b[draw as usize % m.tints_b.len()], "{entity}");
        }
        checked += 1;
    }
    assert!(checked > 20, "{checked} ped entities");
}

#[test]
fn ped_glb_bones_match_the_animation_rig() {
    let Some(bank) = bank() else {
        eprintln!("skipped: needs the ped bank under SKATE3_ASSET_ROOT");
        return;
    };
    let Some(models) = find("private/living_world/models") else {
        eprintln!("skipped: needs private/living_world/models under SKATE3_ASSET_ROOT");
        return;
    };
    let rig = &bank.rig;
    // Reference-pose globals (positions) in the animation space.
    let globals = skate_core::living_world::peds::PedEvaluator::globals(rig, &rig.reference);
    let mut worst = (0.0f32, String::new());
    let mut checked = 0;
    for entry in std::fs::read_dir(&models).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if stem.starts_with("zprop_") || path.extension().is_none_or(|e| e != "glb") {
            continue;
        }
        let (joints, ibm) = glb_skin(&path);
        let (map, unused) = match_bones(&joints, &rig.names);
        let unmatched: Vec<&String> = joints.iter().zip(&map).filter(|(_, m)| m.is_none()).map(|(j, _)| j).collect();
        assert!(unmatched.is_empty(), "{stem}: GLB bones without a rig bone: {unmatched:?}");
        if checked == 0 {
            eprintln!("{stem}: {} GLB bones all match; rig bones the GLB lacks: {unused:?}", joints.len());
        }
        for ((j, m), inv) in joints.iter().zip(&map).zip(&ibm) {
            let i = m.unwrap();
            if !rig.animated[i] {
                continue;
            }
            let bind = inverse_translation(inv);
            let g = globals[i][3];
            let d = ((bind[0] - g[0]).powi(2) + (bind[1] - g[1]).powi(2) + (bind[2] - g[2]).powi(2)).sqrt();
            if d > worst.0 {
                worst = (d, format!("{stem} {j}: bind {bind:?} reference {:?}", &g[..3]));
            }
        }
        checked += 1;
    }
    eprintln!("{checked} ped GLBs; worst bind vs reference distance {:.3} m ({})", worst.0, worst.1);
    assert!(checked >= 50);
    assert!(worst.0 < 0.15, "bind skeletons differ from the animation reference: {}", worst.1);
}

/// fix10 (peds rendered warped): every ped GLB's joint frames ARE the rig's reference-pose frames
/// (rotation and position), so the skin is `bone global x inverse(GLB bind)` with no extra
/// bone-local basis; the skater's basis would turn every bone 90 degrees.
#[test]
fn ped_glb_bind_frames_are_the_rig_reference_frames() {
    let Some(bank) = bank() else {
        eprintln!("skipped: needs the ped bank under SKATE3_ASSET_ROOT");
        return;
    };
    let Some(models) = find("private/living_world/models") else {
        eprintln!("skipped: needs private/living_world/models under SKATE3_ASSET_ROOT");
        return;
    };
    let rig = &bank.rig;
    let mut locals = rig.reference.clone();
    locals[0] = skate_core::living_world::peds::anim::IDENTITY;
    let globals = skate_core::living_world::peds::PedEvaluator::globals(rig, &locals);
    // Angle between the reference global (row-vector native matrix, rows = bone axes) and the
    // Rotation angle between the two frames, axes normalised (the reference carries small bone
    // scales, e.g. hands about 0.996). GLB bind = inverse of the inverse bind = its transpose for
    // the orthonormal bind rotations (column-major array).
    let angle = |native: &[[f32; 4]; 4], ibm: &[f32; 16], basis: [[f32; 3]; 3]| {
        // bind(r, c) = ibm[r * 4 + c]; reference(r, c) = sum_k native[k][r] * basis[k][c] (G x basis).
        let reference = |r: usize, c: usize| (0..3).map(|k| native[k][r] * basis[k][c]).sum::<f32>();
        let n_bind: [f32; 3] = std::array::from_fn(|c| (0..3).map(|r| ibm[r * 4 + c].powi(2)).sum::<f32>().sqrt().max(1e-12));
        let n_ref: [f32; 3] = std::array::from_fn(|c| (0..3).map(|r| reference(r, c).powi(2)).sum::<f32>().sqrt().max(1e-12));
        let trace: f32 = (0..3).flat_map(|r| (0..3).map(move |c| (r, c))).map(|(r, c)| reference(r, c) / n_ref[c] * ibm[r * 4 + c] / n_bind[c]).sum();
        ((trace - 1.0) / 2.0).clamp(-1.0, 1.0).acos().to_degrees()
    };
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let skater = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
    let (mut worst, mut best_skater, mut checked) = ((0.0f32, String::new()), f32::INFINITY, 0);
    for entry in std::fs::read_dir(&models).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if stem.starts_with("zprop_") || path.extension().is_none_or(|e| e != "glb") {
            continue;
        }
        let (joints, ibm) = glb_skin(&path);
        let (map, _) = match_bones(&joints, &rig.names);
        for ((j, m), inv) in joints.iter().zip(&map).zip(&ibm) {
            let Some(i) = *m else { continue };
            if !rig.animated[i] {
                continue;
            }
            let a = angle(&globals[i], inv, identity);
            if a > worst.0 {
                worst = (a, format!("{stem} {j}"));
            }
            best_skater = best_skater.min(angle(&globals[i], inv, skater));
        }
        checked += 1;
    }
    eprintln!("{checked} ped GLBs; worst bind vs reference frame {:.2} deg ({}); with the skater basis at least {best_skater:.1} deg", worst.0, worst.1);
    assert!(checked >= 50);
    assert!(worst.0 < 2.0, "ped GLB bind frames differ from the rig reference: {} deg at {}", worst.0, worst.1);
    assert!(best_skater > 45.0, "the skater basis must not fit the ped GLBs ({best_skater} deg)");
}
