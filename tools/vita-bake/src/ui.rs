//! ui.pak: the original HUD movie and everything it draws, as the
//! prepared data has them: `private/hud/runtime/trickdisplay.json` and the RGBA textures
//! it names (the shape atlases and the Futura font atlases), unchanged. The Vita mounts
//! the pack under the same virtual root as `sim.pak` and the HUD code reads the same
//! relative paths.
use std::path::Path;

use vita_pack::files::{self, File};

const HUD: &str = "private/hud";
const MARKER: &str = "private/session-marker";
const UI: &str = "private/ui";
/// R2: the retail menu movie set the setup exports (`menu-movies/`: manifest, shared language and fonts,
/// the movies and their textures).
const MENU_MOVIES: &str = "private/menu-movies";
/// R2: the menu tables `tools/menu-tables` writes from the user's executable.
const MENU_TABLES: &str = "private/menu-tables.json";
/// R2: the fe sound names the menus register (skate-ui `menu_stage::FE_SOUNDS`), from the audio manifest.
const MENU_SOUNDS: &str = "private/menu-sounds.json";
/// R2: the Trick Guide's data (skate-ui `menu_stage::TRICK_GUIDE_DATA`), built from the skater collections and the
/// menu tables (the Vita does not load the collections a second time for it).
const TRICK_GUIDE: &str = "private/trick-guide.json";
const AUDIO_MANIFEST: &str = "private/audio/audio_manifest.json";
/// R2: the Challenge Map's data (skate-ui `menu_stage::CHALLENGE_MAP_DATA`): its tree, pictures, markers and
/// teleports, from the skater collections, the set's world maps and `teleports.json`.
const CHALLENGE_MAP: &str = "private/challenge-map.json";
/// The retail teleporters (setup `teleporters.py`, research b420 / b421), every volume inline.
const TELEPORTERS: &str = "private/teleporters.json";

fn collect(dir: &Path, rel: &str, out: &mut Vec<File>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let rel = format!("{rel}/{name}");
        if path.is_dir() {
            collect(&path, &rel, out)?;
        } else {
            out.push(File { path: rel, bytes: std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))? });
        }
    }
    Ok(())
}

/// String ids the game passes to the screens itself (not named in the movies).
const PASSED_IN: [&str; 3] = ["ID_REPLAYEDITOR_", "ID_REPLAY_EDITOR_", "ID_COMMON_"];

/// Keeps the strings of a linked screen's `language` table that its movie names (`ID_...` in
/// its actions and characters) or that the game passes in (`PASSED_IN`).
fn trim_language(json: &mut serde_json::Value) {
    let Some(language) = json.get_mut("language").map(std::mem::take) else { return };
    let rest = json.to_string();
    let mut named = std::collections::BTreeSet::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while let Some(at) = rest[i..].find("ID_") {
        let start = i + at;
        let end = start + bytes[start..].iter().take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || **b == b'_').count();
        named.insert(&rest[start..end]);
        i = end.max(start + 3);
    }
    let kept: serde_json::Map<String, serde_json::Value> = language
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| named.contains(k.as_str()) || PASSED_IN.iter().any(|p| k.starts_with(p)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    json["language"] = serde_json::Value::Object(kept);
}

/// The linked screens the game still loads (`skate_ui::menu_runtime`: the manual meter and the trick analyzer).
const LINKED_SCREENS: [&str; 2] = ["manualmeter.linked.json", "trickanalyser.linked.json"];

/// The linked front-end screens (`tools/link_ui_screen.py`, in `SCREENS/runtime/`) and
/// the textures they name, under `private/ui/` in the pack (the movie JSON keeps its
/// texture paths, relative to the prepare folder).
fn screens(dir: &Path, list: &mut Vec<File>) -> Result<(), String> {
    let runtime = dir.join("runtime");
    let mut linked: Vec<_> = std::fs::read_dir(&runtime)
        .map_err(|e| format!("{}: {e}", runtime.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".linked.json")))
        .collect();
    linked.sort();
    for path in linked {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // Only the HUD meters still run as linked screens; the crossbar (R2), the replay editor, its help and the popup
        // (resync b110bab) run on the menu stage from the menu movie set.
        if !LINKED_SCREENS.contains(&name.as_str()) {
            println!("  screen {name}: skipped (the menu stage runs it from the menu movie set)");
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{name}: {e}"))?;
        // Every linked screen carries the game's whole string table (13,607 strings, 1 MB);
        // each keeps only the ids its movie names and the families the game passes in (the
        // replay editor's items and the common buttons). Six resident copies of the
        // whole table fragmented the heap (a park's 17 MB pack could no longer be read).
        let strings_before = json["language"].as_object().map_or(0, |l| l.len());
        trim_language(&mut json);
        let bytes = serde_json::to_vec(&json).map_err(|e| format!("{name}: {e}"))?;
        println!("  screen {name}: {} of {strings_before} strings kept", json["language"].as_object().map_or(0, |l| l.len()));
        let mut wanted: std::collections::BTreeMap<String, u64> = Default::default();
        for shape in json["shapes"].as_object().ok_or("no shapes")?.values().filter_map(|v| v.as_array()).flatten() {
            if let Some(t) = shape["texture"].as_object() {
                let file = t["rgba"].as_str().ok_or("shape texture without a file")?;
                wanted.insert(file.to_owned(), t["width"].as_u64().unwrap_or(0) * t["height"].as_u64().unwrap_or(0) * 4);
            }
        }
        // Font atlases: the size is inside the font definition, so only check that the file
        // is a square RGBA texture (the runtime checks it against the font when loading).
        let mut atlases: Vec<String> = Vec::new();
        for font in json["fonts"].as_object().into_iter().flat_map(|o| o.values()) {
            if let Some(file) = font["texture"].as_str() {
                atlases.push(file.to_owned());
            }
        }
        for (file, size) in &wanted {
            let data = std::fs::read(dir.join(file)).map_err(|e| format!("{name}: {file}: {e}"))?;
            if data.len() as u64 != *size {
                return Err(format!("{name}: {file} is {} bytes, the movie says {size}", data.len()));
            }
            list.push(File { path: format!("{UI}/{file}"), bytes: data });
        }
        for file in &atlases {
            let data = std::fs::read(dir.join(file)).map_err(|e| format!("{name}: font atlas {file}: {e}"))?;
            let side = ((data.len() / 4) as f64).sqrt() as usize;
            if data.is_empty() || side * side * 4 != data.len() {
                return Err(format!("{name}: font atlas {file} is {} bytes, not a square RGBA texture", data.len()));
            }
            list.push(File { path: format!("{UI}/{file}"), bytes: data });
        }
        println!("  screen {name}: {} KiB of movie, {} textures, {} font atlases", bytes.len() / 1024, wanted.len(), atlases.len());
        list.push(File { path: format!("{UI}/{name}"), bytes });
    }
    Ok(())
}

/// The install's teleporters with every volume inline: setup leaves a volume out when it is in the district's own
/// world stream, where the map's `.triggers` sidecar has it (the Vita reads no sidecars). A teleporter whose volume
/// does not resolve is left out.
fn teleporters(assets: &Path) -> Result<Option<Vec<u8>>, String> {
    let Ok(bytes) = std::fs::read(assets.join(TELEPORTERS)) else { return Ok(None) };
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{TELEPORTERS}: {e}"))?;
    let maps = assets.parent().ok_or("assets has no parent")?.join("maps");
    let mut sidecars: std::collections::BTreeMap<String, Vec<skate_data::trigger_volumes::TriggerVolumeRecord>> = Default::default();
    let list = json["teleporters"].as_array_mut().ok_or("teleporters.json: no teleporters")?;
    let before = list.len();
    list.retain_mut(|t| {
        if !t["volume"].is_null() {
            return true;
        }
        let (Some(map), Some(id)) = (t["map"].as_str().map(str::to_owned), t["volume_id"].as_str().map(str::to_owned)) else { return false };
        let volumes = sidecars.entry(map.clone()).or_insert_with(|| {
            std::fs::read(maps.join(format!("{map}.triggers")))
                .ok()
                .and_then(|b| skate_data::trigger_volumes::parse(&b).ok())
                .map(|f| f.volumes)
                .unwrap_or_default()
        });
        match volumes.iter().find(|v| v.id.eq_ignore_ascii_case(&id)) {
            Some(record) => {
                t["volume"] = serde_json::to_value(record).unwrap_or_default();
                true
            }
            None => {
                println!("  teleporter {}: no volume {id} in {map}.triggers, left out", t["id"]);
                false
            }
        }
    });
    println!("  teleporters: {} of {before}", list.len());
    serde_json::to_vec(&json).map(Some).map_err(|e| e.to_string())
}

pub fn bake(assets: &Path, out: &Path, screens_dir: Option<&Path>) -> Result<(), String> {
    let mut list = Vec::new();
    collect(&assets.join(HUD), HUD, &mut list)?;
    // The Trick Guide's demo scene and clips (newer converters export them under `hud/scene`,
    // about 33 MB): the Vita version has no Trick Guide.
    list.retain(|f| !f.path.starts_with(&format!("{HUD}/scene/")));
    collect(&assets.join(MARKER), MARKER, &mut list)?;
    match teleporters(assets) {
        Ok(Some(bytes)) => list.push(File { path: TELEPORTERS.to_owned(), bytes }),
        Ok(None) => println!("  (no teleporters.json: the prepared data predates them)"),
        Err(e) => println!("  (no teleporters: {e})"),
    }
    // R2: the retail menus' movies and tables, when the prepared data has them.
    if assets.join(MENU_MOVIES).is_dir() {
        let before = list.len();
        // The Challenge Map's world maps (14 MB) stay in the pack on the card; the Vita reads one when the map shows it.
        collect(&assets.join(MENU_MOVIES), MENU_MOVIES, &mut list)?;
        println!("  menu movies: {} files", list.len() - before);
    } else {
        println!("  (no menu-movies in the prepared data: no retail menus 2)");
    }
    match std::fs::read(assets.join(MENU_TABLES)) {
        Ok(bytes) => {
            println!("  menu tables: {} KiB", bytes.len() / 1024);
            let tables = std::str::from_utf8(&bytes).map_err(|e| e.to_string()).and_then(skate_data::menu_tables::MenuTables::from_json);
            match (tables, skate_data::collections::Collections::load(assets)) {
                (Ok(tables), Ok(collections)) if !tables.trick_guide.categories.is_empty() => {
                    let guide = skate_data::trick_guide::load(&collections, &tables.trick_guide);
                    let json = serde_json::to_vec(&skate_data::trick_guide::to_json(&guide)).map_err(|e| e.to_string())?;
                    println!("  trick guide: {} categories, {} tricks, {} KiB", guide.category.len(), guide.tricks.len(), json.len() / 1024);
                    list.push(File { path: TRICK_GUIDE.to_owned(), bytes: json });
                    // The Challenge Map: its world maps come with the movie set's manifest.
                    let manifest: serde_json::Value = std::fs::read(assets.join(MENU_MOVIES).join("manifest.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or_default();
                    let world_maps = serde_json::from_value(manifest["world_maps"].clone()).unwrap_or_default();
                    let teleports: serde_json::Value = std::fs::read(assets.join("private/teleports.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or_default();
                    let map = skate_data::challenge_map::bake(&collections, &world_maps, &teleports,
                        &skate_data::challenge_map::LocationUnlocks::load(assets));
                    let json = serde_json::to_vec(&map.to_json()).map_err(|e| e.to_string())?;
                    println!("  challenge map: {} nodes, {} markers, {} teleports, {} KiB", map.nodes.len(), map.markers.len(), map.teleports.len(), json.len() / 1024);
                    list.push(File { path: CHALLENGE_MAP.to_owned(), bytes: json });
                }
                (Err(e), _) | (_, Err(e)) => println!("  (no trick guide: {e})"),
                _ => println!("  (no trick guide: the menu tables have no categories)"),
            }
            list.push(File { path: MENU_TABLES.to_owned(), bytes });
        }
        Err(_) => println!("  (no menu-tables.json: run tools/menu-tables first for the retail menus 2)"),
    }
    // R2: the fe sound records' names (the menus register them at boot); the Vita does not read the 17 MB audio
    // manifest for them.
    if let Ok(bytes) = std::fs::read(assets.join(AUDIO_MANIFEST)) {
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{AUDIO_MANIFEST}: {e}"))?;
        let mut names: Vec<&str> = manifest["frontend"]["sounds"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .filter_map(|s| s["name"].as_str())
            .collect();
        names.sort_unstable();
        println!("  menu sounds: {} names", names.len());
        list.push(File { path: MENU_SOUNDS.to_owned(), bytes: serde_json::to_vec(&names).map_err(|e| e.to_string())? });
    }
    match screens_dir {
        Some(dir) if dir.join("runtime").is_dir() => screens(dir, &mut list)?,
        _ => println!("  (no linked front-end screens: the plain menus only)"),
    }
    // Several screens share font atlases: one copy of each file.
    let mut seen = std::collections::BTreeSet::new();
    list.retain(|f| seen.insert(f.path.clone()));
    // The movie must name only textures that are in the pack, at the sizes it says.
    let movie = list
        .iter()
        .find(|f| f.path == format!("{HUD}/runtime/trickdisplay.json"))
        .ok_or("no private/hud/runtime/trickdisplay.json in the prepared data")?;
    let json: serde_json::Value = serde_json::from_slice(&movie.bytes).map_err(|e| e.to_string())?;
    let mut wanted: Vec<(String, u64)> = Vec::new();
    for shape in json["shapes"].as_object().ok_or("no shapes")?.values().filter_map(|v| v.as_array()).flatten() {
        let t = &shape["texture"];
        wanted.push((t["rgba"].as_str().ok_or("shape texture without a file")?.to_owned(), t["width"].as_u64().unwrap_or(0) * t["height"].as_u64().unwrap_or(0) * 4));
    }
    for font in json["fonts"].as_object().map(|o| o.values().collect::<Vec<_>>()).or_else(|| json["fonts"].as_array().map(|a| a.iter().collect())).unwrap_or_default() {
        if let (Some(file), Some(size)) = (font["texture"].as_str(), font["size"].as_array()) {
            wanted.push((file.to_owned(), size.iter().filter_map(|v| v.as_u64()).product::<u64>() * 4));
        }
    }
    for (file, bytes) in &wanted {
        let path = format!("{HUD}/{file}");
        match list.iter().find(|f| f.path == path) {
            Some(f) if f.bytes.len() as u64 == *bytes => {}
            Some(f) => return Err(format!("{path}: {} bytes, the movie says {bytes}", f.bytes.len())),
            None => return Err(format!("{path} is named by the movie but missing")),
        }
    }
    // The session marker panel: hud.json names its textures (single file names, sizes).
    let marker = list
        .iter()
        .find(|f| f.path == format!("{MARKER}/hud.json"))
        .ok_or("no private/session-marker/hud.json in the prepared data")?;
    let hud: serde_json::Value = serde_json::from_slice(&marker.bytes).map_err(|e| e.to_string())?;
    for t in hud["textures"].as_array().ok_or("no marker textures")? {
        let (file, w, h) = (t["file"].as_str().ok_or("marker texture without a file")?, t["width"].as_u64().unwrap_or(0), t["height"].as_u64().unwrap_or(0));
        let path = format!("{MARKER}/{file}");
        match list.iter().find(|f| f.path == path) {
            Some(f) if f.bytes.len() as u64 == w * h * 4 => {}
            Some(f) => return Err(format!("{path}: {} bytes, the manifest says {}", f.bytes.len(), w * h * 4)),
            None => return Err(format!("{path} is named by the marker HUD but missing")),
        }
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let pack = files::to_pack(&list);
    std::fs::write(out.join("ui.pak"), &pack).map_err(|e| e.to_string())?;
    let raw: usize = list.iter().map(|f| f.bytes.len()).sum();
    println!("{}: {} files ({} textures checked), {} KiB (contents {} KiB)", out.join("ui.pak").display(), list.len(), wanted.len(), pack.len() / 1024, raw / 1024);
    Ok(())
}
