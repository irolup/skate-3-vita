//! `cas.pak`: the character customisation parts (docs/customization-survey.md step 1), from the install's
//! `private/customisation/` (upstream's customiser setup, `tools/asset_pipeline/customiser_setup.py`, at the pinned
//! commit). Layout: `vita_pack::cas`.
//!
//! - Each library model's GLB (one skinned mesh on the stock rig, 22 morph targets) becomes `cas/parts/<id>.part`:
//!   the mesh as the skater bake reads it, its second UV set, and every morph target with only the vertices it moves.
//! - Each base colour the library's materials and tattoos use becomes `cas/textures/<name>.tex`, at most 256 x 256
//!   (`textures::bake`): DXT1, or DXT5 for an alpha material without an opacity image. A hair material's opacity image
//!   is its own texture: retail's `cac_hair_defaultPS` samples its red on the second UV set, apart from
//!   the base colour's alpha (upstream `customiser_material.rs`). The source PNGs are upstream's full-size decodes of
//!   the retail textures, so the small levels are resampled from them, not taken from the retail mip chain.
//! - `cas/rules.bin`: the catalog for the composition rules (`skate_core::cas_rules`).
//! - `cas/vault.json`: the vault rows the screen reads, from the install's `native.json`: colour zones and their colours,
//!   morph limits, presets, preview cameras.
//! - `cas/set.map`: the screen's 3D set (retail `DIST_CreateSakter`, written as `private/customisation/CreateSakter.skate`
//!   by `tools/cas_set.py`), a map pack like the Trick Guide's set; `cas/anim/cac_edit_<gender>.abin`: the stands and
//!   poses the skater plays on it (the install's `stock/data/anim`).
//! - `cas/library.json`: the library with every path replaced by its pack file, and the normal and roughness maps
//!   left out (the Vita's skater shading uses the base colour only).
//! - `cas/thumbs/<name>.tex`: Slappy's thumbnails (`fe/cas/...` as the screen names them), retail's DXT5 blocks
//!   exported by `tools/cas_thumbs.py` (untiled and byte-swapped, not compressed again).
use std::collections::BTreeMap;
use std::path::Path;

use image::GenericImageView;
use serde_json::{json, Value};
use vita_pack::cas::{texture_to_pack, Morph, Part};
use vita_pack::files::File;
use vita_pack::texture::Format;

const DIR: &str = "private/customisation";
/// A morph delta below this moves nothing (the GLBs store zeros).
const NOTHING: f32 = 1e-6;

pub fn bake(install: &Path, out: &Path) -> Result<(), String> {
    let assets = install.join("assets");
    let read_json = |path: &Path| -> Result<Value, String> {
        serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{}: {e} (run upstream's customiser setup)", path.display()))?)
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let current = read_json(&assets.join(DIR).join("current.json"))?;
    let set = current["set"].as_str().ok_or("customisation/current.json has no set")?;
    let mut library = read_json(&assets.join(DIR).join("sets").join(set).join("library-v3.json"))?;
    // The stamps the customiser setup leaves out (tools/cas_stamps.py): the logos, in the library's `logos`.
    if let Ok(extra) = read_json(&assets.join(DIR).join("sets").join(set).join("extra-logos.json")) {
        library["logos"] = extra;
    }
    // And the uncategorised `tattoo_diffuse_N` tattoos.
    if let Ok(extra) = read_json(&assets.join(DIR).join("sets").join(set).join("extra-tattoos.json")) {
        if let (Some(tattoos), Some(extra)) = (library["tattoos"].as_object_mut(), extra.as_object()) {
            tattoos.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    let morph_names = library["morphs"].as_array().ok_or("library has no morphs")?.clone();

    let mut files = Vec::new();
    let mut models = serde_json::Map::new();
    let (mut vertices, mut moved) = (0usize, 0usize);
    for (id, model) in library["models"].as_object().ok_or("library has no models")? {
        let glb = assets.join(model["scene"].as_str().ok_or_else(|| format!("model {id} has no scene"))?);
        let part = part(&glb, morph_names.len()).map_err(|e| format!("model {id}: {e}"))?;
        vertices += part.mesh.vertex_count();
        moved += part.morphs.iter().map(|m| m.vertices.len()).sum::<usize>();
        let path = format!("cas/parts/{id}.part");
        let bytes = part.to_pack();
        if Part::from_pack(&bytes)? != part {
            return Err(format!("model {id}: the part does not read back as written"));
        }
        files.push(File { path: path.clone(), bytes });
        let mut slim = model.clone();
        let o = slim.as_object_mut().unwrap();
        o.remove("scene");
        o.insert("part".into(), json!(path));
        models.insert(id.clone(), slim);
    }
    println!("  parts: {} models, {vertices} vertices, {moved} morph deltas", models.len());

    // The textures: one per base colour (DXT5 when a material cuts it out), and the hair opacity images.
    let mut jobs: BTreeMap<String, (String, bool)> = BTreeMap::new();
    let stem = |p: &str| Path::new(p).file_stem().and_then(|s| s.to_str()).unwrap_or(p).to_owned();
    let mut materials = serde_json::Map::new();
    let mut shaders = serde_json::Map::new();
    for (id, m) in library["materials"].as_object().ok_or("library has no materials")? {
        let diffuse = m["diffuse"].as_str().ok_or_else(|| format!("material {id} has no diffuse"))?;
        let alpha = m["alpha"].as_bool().unwrap_or(false);
        // Hair's coverage is its opacity map; the base colour's alpha is specular data, so it needs none here.
        let cut_out = alpha && m["opacity"].as_str().is_none();
        let name = if cut_out { format!("{}_a", stem(diffuse)) } else { stem(diffuse) };
        jobs.entry(name.clone()).or_insert((diffuse.to_owned(), cut_out));
        let mut entry = json!({
            "name": m["name"], "flags": m["flags"], "alpha": alpha, "tint": m["tint"],
            "texture": format!("cas/textures/{name}.tex"),
        });
        // Retail's character shading of the material (upstream `customiser_lighting.py`: the model's shader family and
        // its nine `m_params` rows).
        // The rows come from the family's vault row, so they are kept once per family (`shaders`).
        if let (Some(shader), Some(rows)) = (m["lighting"]["shader"].as_str(), m["lighting"]["params"].as_array().filter(|r| r.len() == 9)) {
            if shaders.get(shader).is_some_and(|known| known != &Value::Array(rows.clone())) {
                return Err(format!("shader {shader}: materials with other rows"));
            }
            shaders.insert(shader.to_owned(), Value::Array(rows.clone()));
            entry["shader"] = json!(shader);
        }
        if let Some(opacity) = m["opacity"].as_str() {
            let name = format!("{}_op", stem(opacity));
            jobs.entry(name.clone()).or_insert((opacity.to_owned(), false));
            entry["opacity"] = json!(format!("cas/textures/{name}.tex"));
        }
        materials.insert(id.clone(), entry);
    }
    let mut tattoos = serde_json::Map::new();
    for (id, t) in library["tattoos"].as_object().into_iter().flatten() {
        let texture = t["texture"].as_str().ok_or_else(|| format!("tattoo {id} has no texture"))?;
        let name = format!("{}_a", stem(texture));
        jobs.entry(name.clone()).or_insert((texture.to_owned(), true));
        tattoos.insert(id.clone(), json!({ "name": t["name"], "bounds": t["bounds"], "texture": format!("cas/textures/{name}.tex") }));
    }
    let mut logos = serde_json::Map::new();
    for (id, t) in library["logos"].as_object().into_iter().flatten() {
        let texture = t["texture"].as_str().ok_or_else(|| format!("logo {id} has no texture"))?;
        let name = format!("{}_a", stem(texture));
        jobs.entry(name.clone()).or_insert((texture.to_owned(), true));
        logos.insert(id.clone(), json!({ "name": t["name"], "bounds": t["bounds"], "texture": format!("cas/textures/{name}.tex") }));
    }
    let jobs: Vec<(String, (String, bool))> = jobs.into_iter().collect();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let baked: Vec<Result<(String, Vec<u8>, usize), String>> = std::thread::scope(|scope| {
        let chunks: Vec<_> = jobs.chunks(jobs.len().div_ceil(workers).max(1)).collect();
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let assets = &assets;
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(name, (source, alpha))| {
                            let t = texture(assets, source, *alpha).map_err(|e| format!("texture {name}: {e}"))?;
                            let bytes = texture_to_pack(&t);
                            vita_pack::cas::texture_from_pack(&bytes).map_err(|e| format!("texture {name}: {e}"))?;
                            Ok((format!("cas/textures/{name}.tex"), bytes, t.bytes()))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let mut texture_bytes = 0;
    for b in baked {
        let (path, bytes, size) = b?;
        texture_bytes += size;
        files.push(File { path, bytes });
    }
    println!("  textures: {} ({} KiB of texels)", jobs.len(), texture_bytes / 1024);

    // The disc's saved characters, as they are: the default skaters (`SavedRecipeFallback*`) and retail's others.
    let recipes = assets.join("private/stock/data/cacrecipes");
    let mut count = 0;
    for entry in std::fs::read_dir(&recipes).map_err(|e| format!("{}: {e}", recipes.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().and_then(|n| n.to_str()).ok_or("recipe name")?.to_owned();
        let bytes = std::fs::read(&path).map_err(|e| format!("{name}: {e}"))?;
        skate_core::skater_record::SkaterRecord::from_retail(&bytes).map_err(|e| format!("{name}: {e}"))?;
        files.push(File { path: format!("cas/recipes/{name}"), bytes });
        count += 1;
    }
    println!("  recipes: {count}");
    // A record's morph weights are in the vault's target order, not the GLBs' (`morphs`).
    let native = read_json(&assets.join(DIR).join("sets").join(set).join("native.json"))?;
    let mut order: Vec<(u64, &str)> = native["morphs"]
        .as_array()
        .ok_or("native.json has no morphs")?
        .iter()
        .map(|m| Some((m["target_order"].as_u64()?, m["target"].as_str()?)))
        .collect::<Option<_>>()
        .ok_or("native.json: a morph without its order or target")?;
    order.sort_unstable();
    let morph_order: Vec<&str> = order.into_iter().map(|(_, t)| t).collect();

    // The screen's set and the skater's stands: without them the preview draws the skater alone.
    let set_skate = assets.join(DIR).join("CreateSakter.skate");
    match std::fs::read(&set_skate) {
        Ok(skate) => files.push(File { path: "cas/set.map".into(), bytes: crate::map::set_pack(&skate)? }),
        Err(e) => println!("  no set ({}: {e}); run tools/cas_set.py first for the screen's backdrop", set_skate.display()),
    }
    for gender in ["male", "female"] {
        let name = format!("cac_edit_{gender}.abin");
        let bytes = std::fs::read(assets.join("private/stock/data/anim").join(&name)).map_err(|e| format!("{name}: {e}"))?;
        files.push(File { path: format!("cas/anim/{name}"), bytes });
    }
    // Slappy's thumbnails (tools/cas_thumbs.py): retail's DXT5 blocks, one level each, read per page by the screen.
    let thumbs = assets.join(DIR).join("thumbnails");
    match std::fs::read(thumbs.join("thumbnails.json")) {
        Ok(index) => {
            let index: Value = serde_json::from_slice(&index).map_err(|e| format!("thumbnails.json: {e}"))?;
            let (mut count, mut bytes) = (0, 0);
            for t in index["thumbnails"].as_array().ok_or("thumbnails.json has no thumbnails")? {
                let name = t["name"].as_str().ok_or("a thumbnail without a name")?;
                if name.contains("..") {
                    return Err(format!("thumbnail path escapes its folder: {name}"));
                }
                let size = |k: &str| t[k].as_u64().and_then(|v| u16::try_from(v).ok()).ok_or_else(|| format!("thumbnail {name}: no {k}"));
                let blocks = std::fs::read(thumbs.join(format!("{name}.dxt5"))).map_err(|e| format!("thumbnail {name}: {e}"))?;
                let texture = vita_pack::texture::Texture { width: size("width")?, height: size("height")?, format: Format::Dxt5, mips: vec![blocks] };
                texture.validate().map_err(|e| format!("thumbnail {name}: {e}"))?;
                bytes += texture.bytes();
                files.push(File { path: format!("cas/thumbs/{name}.tex"), bytes: texture_to_pack(&texture) });
                count += 1;
            }
            println!("  thumbnails: {count}, {} KiB", bytes / 1024);
        }
        Err(e) => println!("  no thumbnails ({}: {e}); run tools/cas_thumbs.py first for Slappy's pictures", thumbs.display()),
    }
    files.push(File { path: "cas/vault.json".into(), bytes: serde_json::to_vec(&vault(&native)?).map_err(|e| e.to_string())? });
    files.push(File { path: "cas/rules.bin".into(), bytes: rules(&read_json(&assets.join(DIR).join("sets").join(set).join("catalog.json"))?, &native)? });

    let slim = json!({
        "version": 1,
        "set": set,
        "models": models,
        "materials": materials,
        "tattoos": tattoos,
        "logos": logos,
        "defaults": library["defaults"],
        "colours": library["colours"],
        "morphs": morph_names,
        "morph_order": morph_order,
        // The characters' ambient light (upstream `private/character-lighting.json`): nine SH coefficients.
        "shaders": shaders,
        "default_sh": read_json(&assets.join("private/character-lighting.json")).map(|l| l["default_sh"].clone()).unwrap_or(Value::Null),
    });
    let library_bytes = serde_json::to_vec(&slim).map_err(|e| e.to_string())?;
    println!("  library.json: {} KiB", library_bytes.len() / 1024);
    files.push(File { path: "cas/library.json".into(), bytes: library_bytes });

    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let pack = vita_pack::files::to_pack(&files);
    std::fs::write(out.join("cas.pak"), &pack).map_err(|e| e.to_string())?;
    let index = vita_pack::files::index_of(&pack)?;
    std::fs::write(out.join("cas.pak.idx"), vita_pack::files::encode_index(&pack[24..56], &index)).map_err(|e| e.to_string())?;
    println!("{}: {} files, {} KiB", out.join("cas.pak").display(), files.len(), pack.len() / 1024);
    Ok(())
}

/// The rules' catalog (`skate_core::cas_rules`): every LOD 0 row of the retail catalog, and the colours' tags.
/// The screen's vault rows (`skate_vita::cas_look::vault` reads them). Vectors are big-endian floats as hex.
fn vault(native: &Value) -> Result<Value, String> {
    use skate_data::attrib_hash::hash;
    let rows = native["collections"].as_array().ok_or("native.json has no collections")?;
    let class = |name: &'static str| rows.iter().filter(move |r| r["class"] == name);
    let data = |row: &Value, field: &str| row["fields"][field]["data"].as_str().unwrap_or("").to_owned();
    let floats = |text: &str| -> Vec<f32> {
        (0..text.len() / 8).filter_map(|i| u32::from_str_radix(text.get(i * 8..i * 8 + 8)?, 16).ok()).map(f32::from_bits).collect()
    };
    let int = |text: &str| u32::from_str_radix(text, 16).unwrap_or(0);
    let id = |text: &str, at: usize| text.get(at..at + 16).map(str::to_owned).unwrap_or_default();
    let vec3 = |row: &Value, field: &str| {
        let f = floats(&data(row, field));
        json!([f.first().copied().unwrap_or(0.0), f.get(1).copied().unwrap_or(0.0), f.get(2).copied().unwrap_or(0.0)])
    };

    // A colour's zone is its parent row: the zone's reference names it (82541490), sorted by the row's order.
    let mut zones = Vec::new();
    for zone in class("cac_colour_zone") {
        let parent = id(&data(zone, "Hash_9EE25A772ECA5BD4"), 16);
        let mut colours: Vec<_> = class("cac_colour").filter(|c| c["parent"].as_str().is_some_and(|p| format!("{:016X}", hash(p)) == parent)).collect();
        colours.sort_by_key(|c| int(&data(c, "Hash_9C2DC17266699DD9")));
        if zone["key"] == "default" || colours.is_empty() {
            continue;
        }
        zones.push(json!({
            "key": format!("{:016x}", hash(zone["key"].as_str().unwrap_or(""))),
            "slot": int(&data(zone, "Hash_97220B633B540525")),
            "colours": colours.iter().map(|c| json!({
                "key": format!("{:016x}", hash(c["key"].as_str().unwrap_or(""))),
                "name": c["key"],
                "row0": vec3(c, "Hash_97462E6ABC78A423"),
                "swatch": vec3(c, "Hash_7827ED970A88B70C"),
                "row20": vec3(c, "Hash_BC3B0D9B2DB64358"),
                "tag": data(c, "Hash_AB0E65FA2F485ED8"),
            })).collect::<Vec<_>>(),
        }));
    }

    let morphs: Vec<Value> = native["morphs"]
        .as_array()
        .ok_or("native.json has no morphs")?
        .iter()
        .map(|m| json!({"key": m["key"], "index": m["ui_zone"], "min": m["branch_a"]["min"], "max": m["branch_a"]["max"]}))
        .collect();

    // A preset's values are in its rows' arrays; `male` is on its parent row.
    let is_male = |row: &Value| class("cac_presets").find(|p| p["key"] == row["parent"]).map(|p| data(p, "male").starts_with("01")).unwrap_or(false);
    let items = |row: &Value, field: &str| row["fields"][field]["array"]["items"].as_array().cloned().unwrap_or_default();
    let name_of = |key: &str| class("cac_morph_params").find(|m| format!("{:016X}", hash(m["key"].as_str().unwrap_or(""))) == key).and_then(|m| m["key"].as_str()).map(str::to_owned);
    let mut presets = Vec::new();
    for row in class("cac_presets") {
        let key = row["key"].as_str().unwrap_or("");
        if matches!(key, "default" | "male" | "female") {
            continue;
        }
        let morphs: Vec<Value> = items(row, "Hash_9FC1E6E6BACC7ECB")
            .iter()
            .filter_map(|m| {
                let m = m.as_str()?;
                Some(json!([name_of(&id(m, 16))?, floats(m.get(48..56)?).first()?]))
            })
            .collect();
        let colours: Vec<Value> = items(row, "Hash_08C2C7A755AF4140")
            .iter()
            .filter_map(|c| {
                let c = c.as_str()?;
                Some(json!([id(c, 16).to_ascii_lowercase(), id(c, 64).to_ascii_lowercase()]))
            })
            .collect();
        presets.push(json!({
            "key": key,
            "male": is_male(row),
            "number": int(&data(row, "Hash_0DF636FBCE4D41B0")),
            "label": data(row, "Hash_D0D7CE67D4A27FD6"),
            "morphs": morphs,
            "colours": colours,
        }));
    }

    // Each row as its own, its parent's values under it: the rows carry them all on the disc.
    let cameras: Vec<Value> = class("cac_camera")
        .map(|c| {
            json!({
                "key": c["key"],
                "eye": vec3(c, "Hash_F323B5D55CA55A89"),
                "target": vec3(c, "Hash_F0FC2B540F11109E"),
                "fov": floats(&data(c, "Hash_EFB7F7EF35801C19")).first().copied().unwrap_or(35.0),
            })
        })
        .collect();
    // Where the skater stands on the set and how it turns (`cac_body`, read by 82532800 and 8253B618).
    let bodies: Vec<Value> = class("cac_body")
        .map(|b| {
            json!({
                "key": b["key"],
                "position": vec3(b, "Hash_6D2BEA1EFEA33425"),
                "heading": floats(&data(b, "Hash_4591287983616615")).first().copied().unwrap_or(0.0),
                "turn": floats(&data(b, "Hash_165367CDE7FF6727")).first().copied().unwrap_or(0.0),
                "anim": data(b, "Hash_249D0D4624998D68"),
            })
        })
        .collect();
    // The posture's stand (`cac_posture`, 82539688).
    let postures: Vec<Value> = class("cac_posture").map(|p| json!({"key": p["key"], "anim": data(p, "Hash_D32BD93ACFF3E542")})).collect();
    // The unlocks whose items the shop hides until granted (`cac_hidden_unlocks` `default`, 825F35A8): each list holds
    // references (16 hex digits, the referenced row's key hash first) to rows of a class whose key is the unlock name.
    let names = |field: &str, of: &'static str| -> Vec<String> {
        let refs: Vec<String> = class("cac_hidden_unlocks")
            .filter(|r| r["key"] == "default")
            .flat_map(|r| r["fields"][field]["array"]["items"].as_array().cloned().unwrap_or_default())
            .filter_map(|i| i.as_str().and_then(|s| s.get(..16)).map(str::to_ascii_lowercase))
            .collect();
        refs.iter().filter_map(|h| class(of).find(|r| r["key"].as_str().is_some_and(|k| format!("{:016x}", hash(k)) == *h))).filter_map(|r| r["key"].as_str().map(str::to_owned)).collect()
    };
    let hidden = json!({
        "clothing": names("Hash_398276A6376495EE", "cac_unlock_clothing"),
        "sponsors": names("Hash_055CE908E1279D2D", "cac_unlock_sponsors"),
    });
    // The tattoo pages (8253E4D8, 82600AF8 menu 6): the categories in `cac_tattoo_category_order` `default` order, each
    // the `TattooCategory` text its stamps carry and whether stamps without one are listed too; and the stamp zones'
    // cameras (8253E018 > 82536778).
    let tattoo_categories: Vec<Value> = class("cac_tattoo_category_order")
        .filter(|r| r["key"] == "default")
        .flat_map(|r| items(r, "Hash_FFDEC982A33E9230"))
        .filter_map(|i| {
            let h = i.as_str()?.get(..16)?.to_ascii_lowercase();
            let c = class("cac_tattoo_categories").find(|c| c["key"].as_str().is_some_and(|k| format!("{:016x}", hash(k)) == h))?;
            Some(json!({"text": data(c, "Hash_C283F7484D8204B6"), "untagged": data(c, "Hash_4E26F3294AAED961").starts_with("01")}))
        })
        .collect();
    let stamp_zones: Vec<Value> = class("cac_stamp_zone")
        .filter_map(|z| {
            let h = id(&data(z, "Hash_6BCC8ED2FE4DA46F"), 0).to_ascii_lowercase();
            let camera = class("cac_camera").find(|c| c["key"].as_str().is_some_and(|k| format!("{:016x}", hash(k)) == h))?;
            Some(json!({"key": z["key"], "camera": camera["key"]}))
        })
        .collect();
    println!("  vault: {} zones, {} morphs, {} presets, {} cameras, {} bodies, {} tattoo categories, hidden unlocks {hidden}", zones.len(), morphs.len(), presets.len(), cameras.len(), bodies.len(), tattoo_categories.len());
    Ok(json!({"zones": zones, "morphs": morphs, "presets": presets, "cameras": cameras, "bodies": bodies, "postures": postures, "hidden_unlocks": hidden, "tattoo_categories": tattoo_categories, "stamp_zones": stamp_zones}))
}

fn rules(catalog: &Value, native: &Value) -> Result<Vec<u8>, String> {
    use skate_core::skater_record::{Item, Slot};
    let id = |text: &str| u64::from_str_radix(text, 16).map_err(|e| format!("id {text}: {e}"));
    let mut rows = Vec::new();
    for component in catalog["components"].as_array().ok_or("catalog.json has no components")? {
        let name = component["slot"].as_str().ok_or("a component without a slot")?;
        let slot = Slot::from_name(name).ok_or_else(|| format!("unknown slot {name}"))?;
        for model in component["models"].as_array().ok_or("a component without models")? {
            let model_id = id(model["id"].as_str().ok_or("a model without an id")?)?;
            // A row per variant of the first material group only: a later group is a fixed material of another part of
            // the mesh, not a choice (retail's (model, material) table in the disc's `.recipe` holds all 4,593 first-group
            // variants and neither of the two later-group materials, `defaultShadingGroup1` and `lambert1`). [data]
            let groups = model["lods"][0]["material_instances"].as_array().ok_or("a model without material instances")?;
            for variant in groups.first().and_then(|g| g.as_array()).into_iter().flatten() {
                let material = variant["id"].as_str().ok_or("a variant without an id")?;
                let flags = catalog["materials"][material]["flags"].as_object().ok_or_else(|| format!("material {material} has no flags"))?;
                let mut fields: BTreeMap<String, String> = BTreeMap::new();
                for (key, value) in flags.iter().chain(model["flags"].as_object().ok_or("a model without flags")?) {
                    if let (Some(k), Some(v)) = (key.strip_prefix("cas."), value.as_str()) {
                        fields.insert(k.to_owned(), v.to_owned());
                    }
                }
                fields.insert("ModelName".into(), model["name"].as_str().unwrap_or("").to_owned());
                fields.insert("MaterialVariantName".into(), variant["name"].as_str().unwrap_or("").to_owned());
                rows.push((slot, Item { model: model_id, material: id(material)? }, fields));
            }
        }
    }
    let mut tags = Vec::new();
    for row in native["collections"].as_array().ok_or("native.json has no collections")? {
        if row["class"] == "cac_colour" {
            let key = row["key"].as_str().ok_or("a colour without a key")?;
            let tag = row["fields"]["Hash_AB0E65FA2F485ED8"]["data"].as_str().unwrap_or("");
            tags.push((skate_data::attrib_hash::hash(key), tag.to_owned()));
        }
    }
    println!("  rules: {} rows, {} colours", rows.len(), tags.len());
    Ok(skate_core::cas_rules::Catalog::build(&rows, &tags).to_bytes())
}

/// A library model: its mesh (the skater bake's reader), the second UV set and the morph targets, sparse.
fn part(glb: &Path, targets: usize) -> Result<Part, String> {
    let mesh = crate::skinned_model(glb, &|_| false)?;
    let (doc, buffers, _) = gltf::import(glb).map_err(|e| format!("{}: {e}", glb.display()))?;
    let primitive = doc
        .nodes()
        .find(|n| n.mesh().is_some() && n.skin().is_some())
        .and_then(|n| n.mesh())
        .and_then(|m| m.primitives().next())
        .ok_or("no skinned mesh")?;
    if doc.meshes().map(|m| m.primitives().count()).sum::<usize>() != 1 {
        return Err("expected one primitive (the morphs index its vertices)".into());
    }
    let r = primitive.reader(|b| Some(&buffers[b.index()]));
    let uv1: Vec<[f32; 2]> = r.read_tex_coords(1).map(|t| t.into_f32().collect()).unwrap_or_default();
    let mut morphs = Vec::new();
    for (positions, normals, _) in r.read_morph_targets() {
        let positions: Vec<[f32; 3]> = positions.map(|p| p.collect()).unwrap_or_default();
        let normals: Vec<[f32; 3]> = normals.map(|n| n.collect()).unwrap_or_default();
        let mut m = Morph { vertices: Vec::new(), positions: Vec::new(), normals: Vec::new() };
        for (i, p) in positions.iter().enumerate() {
            let n = normals.get(i).copied().unwrap_or([0.0; 3]);
            if p.iter().chain(&n).any(|v| v.abs() > NOTHING) {
                m.vertices.push(u16::try_from(i).map_err(|_| "more than 65536 vertices")?);
                m.positions.push(*p);
                m.normals.push(n);
            }
        }
        morphs.push(m);
    }
    if morphs.len() != targets {
        return Err(format!("{} morph targets, the library names {targets}", morphs.len()));
    }
    let part = Part { mesh, uv1, morphs };
    part.validate()?;
    Ok(part)
}

/// An RGBA PNG: DXT5 with its alpha, else DXT1 (alpha forced opaque).
fn texture(assets: &Path, source: &str, alpha: bool) -> Result<vita_pack::texture::Texture, String> {
    let image = image::open(assets.join(source)).map_err(|e| format!("{source}: {e}"))?;
    let (w, h) = image.dimensions();
    let mut rgba = image.to_rgba8();
    if !alpha {
        rgba.pixels_mut().for_each(|p| p[3] = 255);
    }
    crate::textures::bake(rgba.as_raw(), w, h, if alpha { Format::Dxt5 } else { Format::Dxt1 })
}
