//! `living.pak`, the living world's data (PR #52's `private/living_world/` export) as files
//! under their own paths, read through `skate_data::vfs`:
//!
//! - `tables.json`: the classes the population, the traffic and the props' streaming read (census
//!   records, category groups, census ranges, `livingworld` (DMO weights, signal timings), vehicle
//!   drivers and characteristics); a superset of the one in `props.pak`, mounted after it;
//! - `vehicles.json`: the models (palettes, bounds, wheel size, grab and grind splines, retail
//!   collision mesh), entities, census categories and materials, without the recipes (a model's
//!   collision is taken from its recipe when the model has none);
//! - `roads.bin` and the districts' `<District>.census.bin`, as exported;
//! - `skater_profiles.json` and the districts' NPC skater lines `skater_paths/<District>.bin`,
//!   as exported;
//! - the peds' tables (entity categories, entities, models, animation sets) in their own
//!   `ped_tables.json`, `navmesh.bin`, the
//!   ped animation bank (`PedestrianSkeletonPres.abin`) and every ped model of `livingworld_models`
//!   as a skinned model (`models/<recipe>.ped`: LOD0 and LOD1, the LOD1 materials named `LOD1:...`,
//!   the tinted body materials (retail ped shaders) `TINT:...`, the textures shared).
//! - the behaviour classes in `ped_tables.json` (mood, chase, takedown, sight, conversations, plugin
//!   props, hand props), the compiled ped graphs and plugin descriptors (`.stategraph`, stock paths),
//!   `waypoints.json` and `hand_props.json`.
//! - per car model the census can spawn, its GLB as a skinned model (`vita_pack::skater`, the
//!   glass blended) at `vehicles/<recipe>.car`, the path of the GLB with that extension.
use std::path::Path;
use vita_pack::files::File;

const TABLE_CLASSES: [&str; 6] = [
    "livingworld_census",
    "livingworld_categorygroups",
    "livingworld_census_ranges",
    "livingworld",
    "livingworld_vehicle_drivers",
    "livingworld_vehicle_characteristics",
];
/// The peds' classes go to `ped_tables.json`, read only when the peds load (every
/// living-world loader parses its tables whole: one file with the ped classes cost each of them
/// about 8 MB of parse while it ran).
/// With the behaviour runtime's (mood, chase, takedown, sight, conversations, plugins, hand props).
const PED_TABLE_CLASSES: [&str; 16] = [
    "livingworld_entitycategories",
    "livingworld_entities",
    "livingworld_models",
    "livingworld_entity_animation",
    "livingworld_moodeventcategories",
    "livingworld_entities_moodresults",
    "livingworld_entities_moodreactions",
    "livingworld_entities_chase",
    "livingworld_entity_takedown",
    "livingworld_entities_perceptions",
    "livingworld_conversations",
    "livingworld_conversation_categories",
    "livingworld_conversation_category_groups",
    "livingworld_props",
    "livingworld_handprops",
    "livingworld_handprops_usagecharacteristics",
];
/// The ped behaviour graphs (the AI graph, its plugins) and the plugin descriptors, compiled, as
/// shipped under these stock folders.
const PED_GRAPH_DIRS: [&str; 2] = ["private/stock/data/state/livingworldentities/pedestrian", "private/stock/data/livingworld/PluginDescriptor"];
const MODEL_FIELDS: [&str; 11] =
    ["recipe", "glb", "chassis_colours", "secondary_colours", "palette_ids", "size_hint", "wheel_hint", "mesh_bounds", "grab_splines", "grind_splines", "collision"];
const VEHICLE_KEYS: [&str; 6] = ["version", "entities", "census", "materials", "material_kinds", "environments"];
const DISTRICTS: [&str; 3] = ["DownTown", "Industrial", "University"];

fn json(path: &Path) -> Result<serde_json::Value, String> {
    serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn bake(install: &Path, out: &Path) -> Result<(), String> {
    let dir = install.join("assets/private/living_world");
    let mut files = Vec::new();
    let tables = json(&dir.join("tables.json"))?;
    let mut classes = serde_json::Map::new();
    for class in TABLE_CLASSES {
        classes.insert(class.into(), tables.pointer(&format!("/classes/{class}")).cloned().ok_or(format!("tables.json: no class {class}"))?);
    }
    let slim = serde_json::json!({ "version": tables["version"], "classes": classes });
    files.push(File { path: "private/living_world/tables.json".into(), bytes: serde_json::to_vec(&slim).map_err(|e| e.to_string())? });
    let mut ped_classes = serde_json::Map::new();
    for class in PED_TABLE_CLASSES {
        ped_classes.insert(class.into(), tables.pointer(&format!("/classes/{class}")).cloned().ok_or(format!("tables.json: no class {class}"))?);
    }
    let peds = serde_json::json!({ "version": tables["version"], "classes": ped_classes });
    files.push(File { path: "private/living_world/ped_tables.json".into(), bytes: serde_json::to_vec(&peds).map_err(|e| e.to_string())? });

    let vehicles = json(&dir.join("vehicles.json"))?;
    let mut doc = serde_json::Map::new();
    for key in VEHICLE_KEYS {
        if let Some(v) = vehicles.get(key) {
            doc.insert(key.into(), v.clone());
        }
    }
    // The models the census can spawn (`skate_data::living_world::vehicle_catalog`: every entity of
    // every census category).
    let mut spawnable = std::collections::BTreeSet::new();
    for record in vehicles["census"].as_object().into_iter().flat_map(|m| m.values()) {
        for c in record["categories"].as_array().into_iter().flatten() {
            for e in c["entities"].as_array().into_iter().flatten().filter_map(|e| e.as_str()) {
                if let Some(m) = vehicles["entities"][e]["model"].as_str() {
                    spawnable.insert(m.to_owned());
                }
            }
        }
    }
    let mut models = serde_json::Map::new();
    let mut cars = Vec::new();
    for (key, m) in vehicles["models"].as_object().ok_or("vehicles.json: no models")? {
        let mut kept: serde_json::Map<String, serde_json::Value> = MODEL_FIELDS.iter().filter_map(|k| Some(((*k).to_owned(), m.get(*k)?.clone()))).collect();
        if kept.get("collision").is_none_or(|c| c.is_null()) {
            if let Some(c) = m["recipe"].as_str().map(|r| &vehicles["recipes"][r]["glb"]["collision"]).filter(|c| !c.is_null()) {
                kept.insert("collision".into(), c.clone());
            }
        }
        if spawnable.contains(key) {
            if let Some(glb) = m["glb"].as_str() {
                cars.push(glb.to_owned());
            }
        }
        models.insert(key.clone(), serde_json::Value::Object(kept));
    }
    doc.insert("models".into(), serde_json::Value::Object(models));
    let vehicles_bytes = serde_json::to_vec(&serde_json::Value::Object(doc)).map_err(|e| e.to_string())?;
    println!("  vehicles.json: {} KiB ({} KiB exported), {} spawnable models", vehicles_bytes.len() / 1024, std::fs::metadata(dir.join("vehicles.json")).map_or(0, |m| m.len()) / 1024, spawnable.len());
    files.push(File { path: "private/living_world/vehicles.json".into(), bytes: vehicles_bytes });

    let mut copy = |name: &str| -> Result<(), String> {
        let path = dir.join(name);
        files.push(File { path: format!("private/living_world/{name}"), bytes: std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))? });
        Ok(())
    };
    copy("roads.bin")?;
    copy("skater_profiles.json")?;
    copy("navmesh.bin")?;
    // The plugin waypoint groups and the hand prop models' index.
    copy("waypoints.json")?;
    copy("hand_props.json")?;
    for district in DISTRICTS {
        copy(&format!("{district}.census.bin"))?;
        copy(&format!("skater_paths/{district}.bin"))?;
    }

    // Every compiled graph under the ped behaviour folders (`.stategraph`, their stock paths).
    let mut graphs = 0;
    for root in PED_GRAPH_DIRS {
        let mut stack = vec![install.join("assets").join(root)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|x| x == "stategraph") {
                    let rel = path.strip_prefix(install.join("assets")).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
                    files.push(File { path: rel, bytes: std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))? });
                    graphs += 1;
                }
            }
        }
    }
    println!("  ped behaviour graphs: {graphs}");

    // The ped animation bank under its stock path.
    let bank = install.join("assets").join(skate_data::ped_anim::PED_BANK);
    files.push(File { path: skate_data::ped_anim::PED_BANK.into(), bytes: std::fs::read(&bank).map_err(|e| format!("{}: {e}", bank.display()))? });
    // Every ped model (`livingworld_models` recipes with an exported GLB).
    let mut recipes: Vec<String> = tables
        .pointer("/classes/livingworld_models")
        .and_then(|c| c.as_object())
        .into_iter()
        .flat_map(|c| c.values())
        .filter_map(|m| m.pointer("/fields/recipe").and_then(|r| r.as_str()).map(str::to_owned))
        .collect();
    recipes.sort();
    recipes.dedup();
    let (mut peds, mut ped_bytes) = (0, 0);
    for recipe in &recipes {
        let glb = dir.join(format!("models/{recipe}.glb"));
        if !glb.is_file() {
            continue;
        }
        let model = crate::skinned_model_nodes(
            &glb,
            &|n| match n.name() {
                Some("LOD0") => Some(0),
                Some("LOD1") => Some(1),
                _ => None,
            },
            &|m, lod| {
                let tinted = m.extras().as_ref().and_then(|x| serde_json::from_str::<serde_json::Value>(x.get()).ok()).and_then(|v| v["shader"].as_str().map(skate_core::living_world::peds::colorize::colorized_shader)).unwrap_or(false);
                (format!("{}{}{}", if lod == 1 { "LOD1:" } else { "" }, if tinted { "TINT:" } else { "" }, m.name().unwrap_or("")), false)
            },
        )?;
        let bytes = model.to_pack();
        peds += 1;
        ped_bytes += bytes.len();
        files.push(File { path: format!("private/living_world/models/{recipe}.ped"), bytes });
    }
    println!("  peds: {peds} models of {} recipes, {} KiB", recipes.len(), ped_bytes / 1024);

    // Every hand prop model of `hand_props.json` as a rigid model (`hand_props/<key>.hprop`, the GLB's
    // `LOD0`) and its triangles for the released prop's collision (`hand_props/<key>.tri`: u32 count, then 9 f32 per
    // triangle, in the model's frame).
    let hand_props = json(&dir.join("hand_props.json"))?;
    let (mut hand_count, mut hand_bytes) = (0, 0);
    for (key, prop) in hand_props["props"].as_object().ok_or("hand_props.json: no props")? {
        let Some(file) = prop["file"].as_str() else { continue };
        let glb = dir.join(file);
        let model = crate::skinned_model_nodes(
            &glb,
            &|n| (n.name() == Some("LOD0") && n.mesh().is_some()).then_some(0),
            &|m, _| (m.name().unwrap_or("").to_owned(), false),
        )?;
        let stem = file.strip_suffix(".glb").ok_or(format!("{file}: not a .glb"))?;
        let bytes = model.to_pack();
        hand_bytes += bytes.len();
        files.push(File { path: format!("private/living_world/{stem}.hprop"), bytes });
        files.push(File { path: format!("private/living_world/{stem}.tri"), bytes: rigid_triangles(&glb)? });
        hand_count += 1;
    }
    println!("  hand props: {hand_count} models, {} KiB", hand_bytes / 1024);

    cars.sort();
    cars.dedup();
    for glb in &cars {
        let model = crate::skinned_model(&dir.join(glb), &|m| m.name().is_some_and(|n| n.starts_with("vehicle_glass")))?;
        let bytes = model.to_pack();
        let texture_bytes: usize = model.textures.iter().flat_map(|t| &t.mips).map(Vec::len).sum();
        println!(
            "  {glb}: {} joints, {} triangles, {} primitives, {} textures ({} KiB), {} KiB",
            model.joints.len(),
            model.indices.len() / 3,
            model.primitives.len(),
            model.textures.len(),
            texture_bytes / 1024,
            bytes.len() / 1024
        );
        let path = format!("private/living_world/{}", glb.strip_suffix(".glb").ok_or(format!("{glb}: not a .glb"))?) + ".car";
        files.push(File { path, bytes });
    }

    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let pack = vita_pack::files::to_pack(&files);
    std::fs::write(out.join("living.pak"), &pack).map_err(|e| e.to_string())?;
    let index = vita_pack::files::index_of(&pack)?;
    std::fs::write(out.join("living.pak.idx"), vita_pack::files::encode_index(&pack[24..56], &index)).map_err(|e| e.to_string())?;
    println!("{}: {} files, {} KiB", out.join("living.pak").display(), files.len(), pack.len() / 1024);
    Ok(())
}

/// The triangles of a rigid GLB's `LOD0` mesh in its frame (the node must carry no transform), as `u32` count then
/// 9 `f32` per triangle (a released hand prop's collision, upstream's `mesh_triangles`).
fn rigid_triangles(glb: &Path) -> Result<Vec<u8>, String> {
    let (doc, buffers, _) = gltf::import(glb).map_err(|e| format!("{}: {e}", glb.display()))?;
    let mut out = Vec::new();
    for node in doc.nodes().filter(|n| n.name() == Some("LOD0")) {
        let Some(mesh) = node.mesh() else { continue };
        let m = node.transform().matrix();
        if !(0..4).all(|c| (0..4).all(|r| (m[c][r] - f32::from(u8::from(c == r))).abs() < 1e-5)) {
            return Err(format!("{}: LOD0 has a transform", glb.display()));
        }
        for p in mesh.primitives() {
            let r = p.reader(|b| Some(&buffers[b.index()]));
            let positions: Vec<[f32; 3]> = r.read_positions().ok_or("primitive without positions")?.collect();
            let indices: Vec<u32> = r.read_indices().ok_or("primitive without indices")?.into_u32().collect();
            for t in indices.chunks_exact(3) {
                out.push([positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]]);
            }
        }
    }
    let mut bytes = (out.len() as u32).to_le_bytes().to_vec();
    for t in &out {
        for v in t.iter().flatten() {
            bytes.extend(v.to_le_bytes());
        }
    }
    Ok(bytes)
}
