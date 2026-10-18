//! `vita-bake audio EXPORT OUT`: `sound.pak` (retail audio) from the reference's audio export.
//!
//! EXPORT is the `audio/` folder that the reference's own exporter writes
//! (`skate-3-rust-engine/tools/asset_pipeline/audio_export.py` `convert`, run with the vgmstream
//! r2117 CLI on the build machine): decoded PCM16 WAVs, the AEMS files and
//! `audio_manifest.json`. The pack keeps the board set: the sample banks of [`BANKS`], the
//! rolling grains, the wheel spins, the AEMS projects, MixMap and Splice trees, and the AEMS banks
//! of [`BANKS`]. Every WAV becomes an `.ima` file (4-bit IMA ADPCM, `skate_audio::adpcm`), and the
//! manifest names those. The ambience beds and speech are left out (to be streamed later).
//! Retail-derived output: write it beside the other packs, never into the repository.
use serde_json::Value;
use std::path::Path;
use vita_pack::files::File;

/// The reference's `audio_export.BANKS` (`b3c9679`): the banks the player components play.
pub const BANKS: [&str; 28] = [
    "Skate_Collisions", "Skate_Metal", "sk8_foley", "Sk82_Whsh_Bys", "Sk8_Air_Flip_Tricks", "GRINDS", "board_scrapes",
    "Brd_Squeaks", "WHEEL_SKID_BANK", "FOOT_DRAG", "fstep_skateshoe1_sm", "Bodyslide", "Rolling_Rattles", "Seams_Bank",
    "PatchBank_Rolling_Surfaces", "sense_of_speed", "water_misc", "Foley_Cloth", "PatchBank_SpiderCracks", "PatchBank_Objects",
    "PatchBank_RocksBounce", "Treatments", "water_lapping", "water_lapping_pond", "fountains_waterlaps_left", "water_fountain",
    "ocean_wave_small", "sk8_menu",
];

/// The reference's `audio_export.AEMS_EXTRA_BANKS`: the boot utilities the host posts at start
/// (`c_emitter_utility`, `Start_up_Play_ctl`).
pub const EXTRA_BANKS: [&str; 2] = ["emitter_utility", "Common"];

/// The `.ems` emitter files each retail map loads (the reference's `map_audio::tests::old_ems_files`,
/// read from the stock collections at run time upstream).
pub const MAP_EMS: [(&str, &[&str]); 10] = [
    ("University", &["music_university", "sfx_university", "reverb_university", "speakers_university", "crowds_university"]),
    ("DownTown", &["music_downtown", "sfx_downtown", "reverb_downtown", "speakers_downtown", "crowds_downtown"]),
    ("Industrial", &["music_industrial", "sfx_industrial", "reverb_industrial", "speakers_industrial", "crowds_industrial"]),
    ("DownTownSkatePark", &["sfx_dt_skatepark"]),
    ("IndustrialSkatePark", &["sfx_ind_skatepark"]),
    ("MegaPark", &["sfx_mega_skatepark"]),
    ("MaloofMoneyCup", &["sfx_maloof_money_cup"]),
    ("StartPark", &["sfx_startpark"]),
    ("BlackBoxPark", &["sfx_blackbox_park"]),
    ("SkateSchool", &["skateschool"]),
];

/// The banks the maps' sound emitters play (`eVolumeType` 1 records with a bank): 100 banks, 64 MiB
/// of ADPCM and 21 MiB of bank programs. They go to `world.pak`, which stays on the card and is
/// read by proximity; `sound.pak` keeps their manifest entries.
fn world_banks(manifest: &Value) -> std::collections::BTreeSet<String> {
    let mut banks = std::collections::BTreeSet::new();
    for (_, groups) in MAP_EMS {
        for group in groups {
            let records = manifest.get("emitters").and_then(|e| e.get(*group)).and_then(Value::as_array);
            for r in records.into_iter().flatten() {
                if r.get("kind").and_then(Value::as_i64) == Some(1) && r.get("flags").and_then(Value::as_i64) == Some(0) {
                    if let Some(b) = r.get("bank").and_then(Value::as_str) {
                        banks.insert(b.to_owned());
                    }
                }
            }
        }
    }
    // The zone crossfades' banks (`Main_Ambience_Crossfade_*`): looping transition sounds, 0.2-0.5 MiB each.
    banks.extend(["Main_Ambience_Crossfade_DT", "Main_Ambience_Crossfade_Ind", "Main_Ambience_Crossfade_Uni"].map(str::to_owned));
    banks
}

/// Planar PCM16 channels and the rate of a PCM16 WAV.
fn wav(bytes: &[u8]) -> Result<(u32, Vec<Vec<i16>>), String> {
    let u16le = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32le = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV".into());
    }
    let (mut at, mut format) = (12, None);
    while at + 8 <= bytes.len() {
        let size = u32le(at + 4) as usize;
        let body = at + 8;
        match &bytes[at..at + 4] {
            b"fmt " => format = Some((u16le(body), u16le(body + 2) as usize, u32le(body + 4), u16le(body + 14))),
            b"data" => {
                let Some((1, channels, rate, 16)) = format else { return Err("not a PCM16 WAV".into()) };
                let data = &bytes[body..(body + size).min(bytes.len())];
                let mut planar = vec![Vec::with_capacity(data.len() / 2 / channels.max(1)); channels];
                for (i, s) in data.chunks_exact(2).enumerate() {
                    planar[i % channels].push(i16::from_le_bytes([s[0], s[1]]));
                }
                return Ok((rate, planar));
            }
            _ => {}
        }
        at = body + size + (size & 1);
    }
    Err("WAV without data".into())
}

/// Every `"file": "….wav"` under `value`, renamed to `.ima` in place; returns the WAV paths.
fn retarget(value: &mut Value, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == "file" {
                    if let Value::String(s) = v {
                        if let Some(stem) = s.strip_suffix(".wav") {
                            found.push(s.clone());
                            *s = format!("{stem}.ima");
                        }
                    }
                } else {
                    retarget(v, found);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| retarget(v, found)),
        _ => {}
    }
}

pub fn bake(export: &Path, out: &Path) -> Result<(), String> {
    let started = std::time::Instant::now();
    let read = |rel: &str| std::fs::read(export.join(rel)).map_err(|e| format!("{rel}: {e}"));
    let mut manifest: Value = serde_json::from_slice(&read("audio_manifest.json")?).map_err(|e| e.to_string())?;
    let obj = manifest.as_object_mut().ok_or("audio_manifest.json is not an object")?;
    // The board set: drop the other banks, the ambience beds and the decoded speech. The
    // samples of every Splice tree stay too (the trees play them: `CellPhone_Rings` is the
    // session marker's phone UI).
    let splice: Vec<String> = obj.get("aems").and_then(|a| a.get("splice")).and_then(Value::as_object).map_or(Vec::new(), |m| m.keys().cloned().collect());
    let world = world_banks(&Value::Object(obj.clone()));
    let board = |k: &str| BANKS.contains(&k) || EXTRA_BANKS.contains(&k) || splice.iter().any(|s| s == k);
    if let Some(Value::Object(banks)) = obj.get_mut("banks") {
        banks.retain(|k, _| board(k) || world.contains(k));
    }
    obj.insert("ambience".into(), Value::Object(Default::default()));
    if let Some(Value::Object(speech)) = obj.get_mut("speech") {
        for entry in speech.values_mut() {
            if let Some(e) = entry.as_object_mut() {
                e.insert("audio".into(), Value::Null);
            }
        }
    }
    let mut copied: Vec<String> = Vec::new();
    if let Some(Value::Object(aems)) = obj.get_mut("aems") {
        if let Some(Value::Object(banks)) = aems.get_mut("banks") {
            banks.retain(|k, _| board(k) || world.contains(k));
        }
        let mut strings = Vec::new();
        collect_strings(&Value::Object(aems.clone()), &mut strings);
        copied.extend(strings.into_iter().filter(|s| s.starts_with("aems/")));
    }
    let mut wavs = Vec::new();
    retarget(&mut manifest, &mut wavs);
    // The world banks (emitters) go to world.pak: their samples and their bank programs.
    let world_only: Vec<&String> = world.iter().filter(|b| !board(b)).collect();
    let world_abk: std::collections::BTreeSet<String> = world_only
        .iter()
        .filter_map(|b| manifest.get("aems")?.get("banks")?.get(b.as_str())?.as_str().map(str::to_owned))
        .collect();
    let in_world = |rel: &str| world_only.iter().any(|b| rel.starts_with(&format!("banks/{b}/")));
    let mut files = Vec::new();
    let mut world_files = Vec::new();
    let (mut pcm_bytes, mut ima_bytes) = (0usize, 0usize);
    for rel in &wavs {
        let bytes = read(rel)?;
        let (rate, planar) = wav(&bytes).map_err(|e| format!("{rel}: {e}"))?;
        let ima = skate_audio::adpcm::to_ima(rate, &planar);
        let file = File { path: format!("private/audio/{}.ima", rel.strip_suffix(".wav").unwrap()), bytes: ima };
        if in_world(rel) {
            world_files.push(file);
        } else {
            pcm_bytes += bytes.len();
            ima_bytes += file.bytes.len();
            files.push(file);
        }
    }
    copied.sort();
    copied.dedup();
    for rel in &copied {
        let file = File { path: format!("private/audio/{rel}"), bytes: read(rel)? };
        if world_abk.contains(rel) {
            world_files.push(file);
        } else {
            files.push(file);
        }
    }
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    files.push(File { path: "private/audio/audio_manifest.json".into(), bytes: manifest_bytes });
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let pack = vita_pack::files::to_pack(&files);
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let path = out.join("sound.pak");
    std::fs::write(&path, &pack).map_err(|e| e.to_string())?;
    write_index(&path, &pack)?;
    // Read back: every file present and every .ima parses.
    let back = vita_pack::files::from_pack(&std::fs::read(&path).map_err(|e| e.to_string())?)?;
    for (p, b) in &back {
        if p.ends_with(".ima") {
            skate_audio::adpcm::pcm_from_ima(b).map_err(|e| format!("{p}: {e}"))?;
        }
    }
    if !world_files.is_empty() {
        world_files.sort_by(|a, b| a.path.cmp(&b.path));
        let pack = vita_pack::files::to_pack(&world_files);
        let path = out.join("world.pak");
        std::fs::write(&path, &pack).map_err(|e| e.to_string())?;
        write_index(&path, &pack)?;
        for (p, b) in vita_pack::files::from_pack(&std::fs::read(&path).map_err(|e| e.to_string())?)? {
            if p.ends_with(".ima") {
                skate_audio::adpcm::pcm_from_ima(&b).map_err(|e| format!("{p}: {e}"))?;
            }
        }
        println!("{}: {:.1} MiB; {} banks, {} files (emitters: read by proximity from the card)", path.display(), pack.len() as f64 / 1048576.0, world_only.len(), world_files.len());
    }
    println!(
        "{}: {:.1} MiB; {} samples ({:.1} MiB PCM16 -> {:.1} MiB ADPCM), {} AEMS files [{:.1} s]",
        path.display(),
        pack.len() as f64 / 1048576.0,
        wavs.len(),
        pcm_bytes as f64 / 1048576.0,
        ima_bytes as f64 / 1048576.0,
        copied.len(),
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

/// Writes `<pack>.idx` next to a pack just written (its index, so the Vita does not read the whole
/// pack to index it on first launch).
fn write_index(path: &Path, pack: &[u8]) -> Result<(), String> {
    let index = vita_pack::files::index_of(pack)?;
    let idx = format!("{}.idx", path.display());
    std::fs::write(&idx, vita_pack::files::encode_index(&pack[24..56], &index)).map_err(|e| format!("{idx}: {e}"))
}

/// `ambience.pak`: the zone beds (`ambience/<bed>.wav` of the export) as `.ima` files named
/// `private/audio/ambience/<bed>.ima` (the layout of sound.pak). The Vita keeps this pack on the card and streams the beds from it
/// (`skate_audio::stream`); 137 MB for the 22 beds.
pub fn bake_ambience(export: &Path, out: &Path) -> Result<(), String> {
    let started = std::time::Instant::now();
    let dir = export.join("ambience");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".wav").map(str::to_owned))
        .collect();
    names.sort();
    let (mut pcm_bytes, mut files) = (0usize, Vec::new());
    for name in &names {
        let bytes = std::fs::read(dir.join(format!("{name}.wav"))).map_err(|e| format!("{name}: {e}"))?;
        let (rate, planar) = wav(&bytes).map_err(|e| format!("{name}: {e}"))?;
        pcm_bytes += bytes.len();
        files.push(File { path: format!("private/audio/ambience/{name}.ima"), bytes: skate_audio::adpcm::to_ima(rate, &planar) });
    }
    let pack = vita_pack::files::to_pack(&files);
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let path = out.join("ambience.pak");
    std::fs::write(&path, &pack).map_err(|e| e.to_string())?;
    write_index(&path, &pack)?;
    // Read back: every bed parses.
    for (p, b) in vita_pack::files::from_pack(&std::fs::read(&path).map_err(|e| e.to_string())?)? {
        skate_audio::adpcm::pcm_from_ima(&b).map_err(|e| format!("{p}: {e}"))?;
    }
    println!(
        "{}: {:.1} MiB; {} beds ({:.1} MiB PCM16) [{:.1} s]",
        path.display(),
        pack.len() as f64 / 1048576.0,
        names.len(),
        pcm_bytes as f64 / 1048576.0,
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

fn collect_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        Value::Object(m) => m.values().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}
