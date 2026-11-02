//! `sim.pak` and `anim.pak`: the files the production load reads, as
//! listed by tracing `sim-oracle run` and proven complete by `sim-oracle run-live
//! --packs` in strict mode. The three converted JSON files go in their binary form
//! (`skate_data::binary`), checked here against the JSON through the real loaders.
use std::path::Path;
use std::sync::Arc;

use skate_data::{collections::Collections, physics_skeleton::PhysicsSkeleton, vfs, GameAssets};
use vita_pack::files::{self, File};

const COLLECTIONS: &str = "private/stock/skater-collections.json";
const SKELETONS: &str = "private/stock/physics-skeletons.json";
const MANIFEST: &str = "private/game.json";
/// Only checked for existence by the manifest (the render reads skater.pak).
const CHARACTER: &str = "private/skater.glb";
/// Read as-is by their loaders.
const VERBATIM: [&str; 13] = [
    "private/stock/data/camera/1.shk",
    "private/stock/data/camera/2.shk",
    "private/stock/data/joystick/skater.pat",
    "private/stock/data/joystick/skater90.pat",
    "private/stock/data/joystick/skaterN90.pat",
    "private/stock/data/joystick/skater_air.pat",
    "private/stock/data/joystick/skater_fingerflip.pat",
    "private/stock/data/joystick/skaterls.pat",
    "private/stock/data/joystick/skaterstep.pat",
    "private/stock/data/script/camera/Default_cameragraph.stategraph",
    "private/stock/data/state/ActionGraph_OnBoard.stategraph",
    "private/stock/data/state/MotionGraph_OnBoard.stategraph",
    "private/stock/data/config/input.cfg",
];
/// English trick and grind names for the HUD, from the HUD runtime's language
/// table: `ID<TAB>Text` lines.
pub const TRICK_NAMES: &str = "private/hud/trick-names.txt";
const HUD_RUNTIME: &str = "private/hud/runtime/trickdisplay.json";

fn trick_names(assets: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(assets.join(HUD_RUNTIME)).map_err(|e| format!("{HUD_RUNTIME}: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let language = v["language"].as_object().ok_or("no language table in the HUD runtime")?;
    let mut lines: Vec<String> = language
        .iter()
        .filter(|(k, _)| k.starts_with("ID_TRICK_") || k.starts_with("GRIND_"))
        .filter_map(|(k, t)| t.as_str().map(|t| format!("{k}\t{}", t.replace(['\t', '\n'], " "))))
        .collect();
    lines.sort();
    println!("  trick names: {}", lines.len());
    Ok((lines.join("\n") + "\n").into_bytes())
}

const BANKS: [&str; 2] = ["private/stock/data/anim/OnBoard.abin", "private/stock/data/anim/OffBoard.abin"];

pub fn bake(assets: &Path, out: &Path) -> Result<(), String> {
    let read = |rel: &str| std::fs::read(assets.join(rel)).map_err(|e| format!("{rel}: {e}"));
    let mut sim = vec![
        File { path: MANIFEST.into(), bytes: GameAssets::encode_json(std::str::from_utf8(&read(MANIFEST)?).map_err(|e| e.to_string())?).map_err(|e| e.0)? },
        File { path: CHARACTER.into(), bytes: Vec::new() },
        File { path: COLLECTIONS.into(), bytes: skate_data::collections::encode_json(&read(COLLECTIONS)?)? },
        File { path: SKELETONS.into(), bytes: skate_data::physics_skeleton::encode_json(&read(SKELETONS)?)? },
    ];
    sim.push(File { path: TRICK_NAMES.into(), bytes: trick_names(assets)? });
    for rel in VERBATIM {
        if assets.join(rel).is_file() {
            sim.push(File { path: rel.into(), bytes: read(rel)? });
        } else {
            println!("  (absent, not packed: {rel})");
        }
    }
    let anim: Vec<File> = BANKS.iter().map(|rel| Ok(File { path: (*rel).into(), bytes: read(rel)? })).collect::<Result<_, String>>()?;
    verify(assets, &sim)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for (name, list) in [("sim.pak", &sim), ("anim.pak", &anim)] {
        let pack = files::to_pack(list);
        std::fs::write(out.join(name), &pack).map_err(|e| e.to_string())?;
        let raw: usize = list.iter().map(|f| f.bytes.len()).sum();
        println!("{}: {} files, {} KiB (contents {} KiB)", out.join(name).display(), list.len(), pack.len() / 1024, raw / 1024);
    }
    Ok(())
}

/// The binary forms, mounted under a virtual root, must load to the same values as
/// the JSON files through the production loaders.
fn verify(assets: &Path, sim: &[File]) -> Result<(), String> {
    let root = Path::new("vfs-bake-verify:/assets");
    for f in sim {
        vfs::mount(root.join(&f.path), Arc::from(f.bytes.as_slice()));
    }
    let json = Collections::load(assets)?;
    let binary = Collections::load(root)?;
    // The loading screen's hint pairs are not in sim.pak's form (`collections::Raw::decode`; the ui bake reads them from
    // the JSON): compared without them.
    let without_pairs = |entries: &[skate_data::collections::Collection]| -> Vec<skate_data::collections::Collection> {
        let mut entries = entries.to_vec();
        for f in entries.iter_mut().flat_map(|c| c.fields.values_mut()) {
            if let Some(a) = &mut f.array {
                a.text_pairs.clear();
            }
        }
        entries
    };
    let (a, b) = (without_pairs(json.entries()), binary.entries().to_vec());
    if a.len() != b.len() {
        return Err(format!("binary collections differ from the JSON: {} entries, {} in the JSON", b.len(), a.len()));
    }
    if let Some((x, y)) = a.iter().zip(b.iter()).map(|(x, y)| (format!("{x:?}"), format!("{y:?}"))).find(|(x, y)| x != y) {
        let at = x.bytes().zip(y.bytes()).position(|(p, q)| p != q).unwrap_or(0);
        let around = |s: &str| s.get(at.saturating_sub(80)..(at + 120).min(s.len())).unwrap_or_default().to_owned();
        return Err(format!("binary collections differ from the JSON: JSON ...{}... binary ...{}...", around(&x), around(&y)));
    }
    // Every skeleton in the file, through the loader (which selects one by name).
    let names: Vec<String> = serde_names(&std::fs::read(assets.join(SKELETONS)).map_err(|e| e.to_string())?)?;
    let sha = serde_sha(&std::fs::read(assets.join(SKELETONS)).map_err(|e| e.to_string())?)?;
    for name in &names {
        let a = PhysicsSkeleton::load(&assets.join(SKELETONS), &sha, name)?;
        let b = PhysicsSkeleton::load(&root.join(SKELETONS), &sha, name)?;
        if format!("{a:?}") != format!("{b:?}") {
            return Err(format!("binary physics skeleton {name} differs from the JSON"));
        }
    }
    let (a, b) = (GameAssets::load(assets).map_err(|e| e.0)?, GameAssets::load(root).map_err(|e| e.0)?);
    if format!("{a:?}") != format!("{b:?}") {
        return Err("binary game manifest differs from the JSON".into());
    }
    println!("  verified: {} collections, {} physics skeletons, manifest identical to the JSON", json.entries().len(), names.len());
    vfs::unmount_all();
    Ok(())
}

fn serde_names(json: &[u8]) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    Ok(v["skeletons"].as_array().ok_or("no skeletons")?.iter().filter_map(|s| s["name"].as_str().map(String::from)).collect())
}
fn serde_sha(json: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    v["source_sha256"].as_str().map(String::from).ok_or_else(|| "no source_sha256".into())
}
