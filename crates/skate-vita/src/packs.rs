//! The baked packs on the memory card: `sim.pak` and `anim.pak`
//! mounted as the loaders' files, `map_<park>.pak` per retail park.
use std::path::Path;

use vita_pack::map::{Map, Render};

pub const DIR: &str = "ux0:data/SKATEVITA";
/// The asset root the loaders see once the packs are mounted.
pub const ROOT: &str = "vfs:/assets";
/// The unpacked converted assets (sim-bench's layout), used if the packs are missing.
pub const FALLBACK: &str = "ux0:data/SKATEVITA/assets";

/// Selectable worlds: the procedural Course, then the baked parks.
/// Selectable worlds: the Course, the seven parks, then the three campaign districts.
pub const WORLDS: [&str; 11] = [
    "Course",
    "StartPark",
    "MegaPark",
    "BlackBoxPark",
    "IndustrialSkatePark",
    "DownTownSkatePark",
    "MaloofMoneyCup",
    "SkateSchool",
    "Industrial",
    "University",
    "DownTown",
];
/// How many of `WORLDS` are the Course and the parks (the rest are districts).
pub const PARKS: usize = 8;

/// Set when ui.pak (the original HUD movie and textures) was mounted.
pub static UI: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Set when `mount` has finished (packs mounted, or the unpacked fallback chosen): the
/// UI loaders can start without waiting for the first session.
pub static MOUNTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set when a pack's background hash check failed (shown on screen).
pub static DAMAGED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Mounts sim.pak and anim.pak (strict: the loaders read nothing else). Returns the
/// asset root to load from, or the fallback folder when the packs are absent.
///
/// The packs stay on the card: each file is read from there when a loader asks for it (since the
/// retail audio, the 38 MB they took in memory are needed elsewhere). The payload hashes are
/// checked on another thread while the session loads; a mismatch sets [`DAMAGED`].
pub fn mount() -> &'static str {
    let started = std::time::Instant::now();
    let mut mounted = 0;
    let mut checked = Vec::new();
    let mut add = |name: &'static str| -> Result<(), String> {
        let path = format!("{DIR}/{name}");
        let (_, _, entries) = pack_index(&path)?;
        let pack = std::sync::Arc::new(std::path::PathBuf::from(&path));
        for (rel, offset, len) in entries {
            skate_data::vfs::mount_disk(Path::new(ROOT).join(rel), pack.clone(), offset, len);
            mounted += 1;
        }
        checked.push((name, path));
        Ok(())
    };
    for name in ["sim.pak", "anim.pak"] {
        if let Err(e) = add(name) {
            println!("{DIR}/{name}: {e}; loading the unpacked assets from {FALLBACK}");
            MOUNTED.store(true, std::sync::atomic::Ordering::Relaxed);
            return FALLBACK;
        }
    }
    // The movable props (optional: without props.pak a world has none).
    if let Err(e) = add("props.pak") {
        println!("{DIR}/props.pak: {e}; no movable props");
    }
    // The living world (traffic); after props.pak, whose living-world tables it extends.
    if let Err(e) = add("living.pak") {
        println!("{DIR}/living.pak: {e}; no traffic");
    }
    // The original HUD is optional: without ui.pak the basic HUD is drawn.
    match add("ui.pak") {
        Ok(()) => UI.store(true, std::sync::atomic::Ordering::Relaxed),
        Err(e) => println!("{DIR}/ui.pak: {e}; the basic HUD is used"),
    }
    skate_data::vfs::set_strict(true);
    println!("packs mounted (on the card): {mounted} files in {:.1} s", started.elapsed().as_secs_f32());
    std::thread::Builder::new()
        .name("pack-verify".into())
        .spawn(move || {
            let _ = vita_platform::vita::pin_current_thread(2);
            // Below the window builder (170): hashing must not starve a collision window.
            vita_platform::vita::set_current_thread_priority(190);
            let started = std::time::Instant::now();
            for (name, path) in checked {
                if let Err(e) = vita_pack::verify_file(Path::new(&path)) {
                    println!("{name}: {e}");
                    *DAMAGED.lock().unwrap() = Some(format!("{name} is damaged: upload the packs again"));
                }
            }
            println!("packs verified in {:.1} s", started.elapsed().as_secs_f32());
        })
        .expect("spawn the pack check");
    MOUNTED.store(true, std::sync::atomic::Ordering::Relaxed);
    ROOT
}

/// Mounts every file of the file pack at `path` under `root` as a file on the card (`vfs::mount_disk`:
/// read on access, or streamed by offset). Returns the number of files.
pub fn mount_on_card(path: &str, root: &str) -> Result<usize, String> {
    let (_, _, entries) = pack_index(path)?;
    let pack = std::sync::Arc::new(std::path::PathBuf::from(path));
    let count = entries.len();
    for (name, offset, len) in entries {
        skate_data::vfs::mount_disk(Path::new(root).join(name), pack.clone(), offset, len);
    }
    Ok(count)
}

/// The cached index of a pack (`mount_selected`): its payload hash, then (name, offset, length)
/// per file. None when absent, unreadable or for another pack.
fn read_index(cache: &str, hash: &[u8]) -> Option<Vec<(String, u64, usize)>> {
    vita_pack::files::decode_index(&std::fs::read(cache).ok()?, hash)
}

fn write_index(cache: &str, hash: &[u8], entries: &[(String, u64, usize)]) {
    if let Err(e) = std::fs::write(cache, vita_pack::files::encode_index(hash, entries)) {
        println!("{cache}: {e} (the pack is indexed again next time)");
    }
}

/// A file pack's index: (name, offset, length) per file, from the cache next to the pack (keyed by
/// the payload hash) or one pass over the pack. Also the open file and its size.
fn pack_index(path: &str) -> Result<(std::fs::File, u64, Vec<(String, u64, usize)>), String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    // Pass 1 reads straight through (a seek per entry drops the buffer, and 6000 small card reads
    // took over 30 s on the Vita).
    let mut r = std::io::BufReader::with_capacity(64 * 1024, file.try_clone().map_err(|e| e.to_string())?);
    let mut header = [0u8; vita_pack::HEADER_LEN];
    r.read_exact(&mut header).map_err(|e| format!("{path}: {e}"))?;
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    if header[..8] != vita_pack::MAGIC || word(8) != 2 || word(12) != vita_pack::files::VERSION {
        return Err("not a file pack of this version".into());
    }
    if u64::from_le_bytes(header[16..24].try_into().unwrap()) + vita_pack::HEADER_LEN as u64 != size {
        return Err("pack length does not match its header (truncated upload?)".into());
    }
    let err = |e: std::io::Error| format!("{path}: {e}");
    let mut u32_at = |r: &mut std::io::BufReader<std::fs::File>| -> Result<u32, String> {
        let mut b = [0u8; 4];
        r.read_exact(&mut b).map_err(err)?;
        Ok(u32::from_le_bytes(b))
    };
    // Pass 1: the index (name, offset, length), skipping the data; cached next to the pack under
    // its payload hash, so later boots read only the data.
    let cache = format!("{path}.idx");
    let entries = match read_index(&cache, &header[24..56]) {
        Some(e) => e,
        None => {
            let count = u32_at(&mut r)?;
            let mut entries = Vec::with_capacity(count as usize);
            let mut at = vita_pack::HEADER_LEN as u64 + 4;
            for _ in 0..count {
                let mut len = [0u8; 2];
                r.read_exact(&mut len).map_err(err)?;
                let mut name = vec![0u8; usize::from(u16::from_le_bytes(len))];
                r.read_exact(&mut name).map_err(err)?;
                let name = String::from_utf8(name).map_err(|e| e.to_string())?;
                let data = u32_at(&mut r)? as u64;
                at += 2 + name.len() as u64 + 4;
                if at + data > size {
                    return Err("pack truncated".into());
                }
                std::io::copy(&mut (&mut r).take(data), &mut std::io::sink()).map_err(err)?;
                entries.push((name, at, data as usize));
                at += data;
            }
            write_index(&cache, &header[24..56], &entries);
            entries
        }
    };
    if entries.iter().any(|(_, offset, len)| offset + *len as u64 > size) {
        return Err("pack index does not match the pack".into());
    }
    Ok((file, size, entries))
}

/// Mounts the file pack at `path` under `root` for the retail audio (`sound.pak`): the files
/// `keep` accepts are read into one buffer and mounted as ranges of it (3000 files, one
/// allocation); the `temporary` ones get buffers of their own, so that unmounting them after
/// loading frees them; the rest are not read. The payload hash is not checked; the lengths are.
/// Returns (files kept, bytes resident).
pub fn mount_selected(
    path: &str,
    root: &str,
    keep: impl Fn(&str) -> bool,
    temporary: impl Fn(&str) -> bool,
) -> Result<(usize, usize), String> {
    use std::io::{Read, Seek, SeekFrom};
    let _foreground = skate_data::io_gate::foreground();
    let (file, _size, entries) = pack_index(path)?;
    let err = |e: std::io::Error| format!("{path}: {e}");
    // Pass 2: runs of consecutive resident files into one buffer, one read per run (the entry
    // headers between them come along); the temporary files into buffers of their own.
    let mut r = file;
    let resident_kind = |n: &str| keep(n) && !temporary(n);
    let mut runs: Vec<(u64, u64, Vec<usize>)> = Vec::new();
    for (i, (name, offset, len)) in entries.iter().enumerate() {
        if !resident_kind(name) {
            continue;
        }
        let end = offset + *len as u64;
        match runs.last_mut() {
            Some(run) if i > 0 && run.2.last() == Some(&(i - 1)) => {
                run.1 = end;
                run.2.push(i);
            }
            _ => runs.push((*offset, end, vec![i])),
        }
    }
    let resident: usize = runs.iter().map(|(a, b, _)| (b - a) as usize).sum();
    // SAFETY: zeroed bytes are valid u8.
    let mut buffer = unsafe { std::sync::Arc::<[u8]>::new_zeroed_slice(resident).assume_init() };
    let mut mounts = Vec::new();
    {
        let bytes = std::sync::Arc::get_mut(&mut buffer).expect("new buffer");
        let mut fill = 0usize;
        for (start, end, members) in &runs {
            let span = (end - start) as usize;
            r.seek(SeekFrom::Start(*start)).map_err(err)?;
            for chunk in bytes[fill..fill + span].chunks_mut(1024 * 1024) {
                r.read_exact(chunk).map_err(err)?;
            }
            for &i in members {
                let (name, offset, len) = &entries[i];
                let at = fill + (offset - start) as usize;
                mounts.push((name.clone(), at..at + len));
            }
            fill += span;
        }
        for (name, offset, len) in &entries {
            if keep(name) && temporary(name) {
                r.seek(SeekFrom::Start(*offset)).map_err(err)?;
                let mut own = vec![0u8; *len];
                r.read_exact(&mut own).map_err(err)?;
                skate_data::vfs::mount(Path::new(root).join(name), own.into());
            }
        }
    }
    let kept = mounts.len();
    for (name, range) in mounts {
        skate_data::vfs::mount_range(Path::new(root).join(name), buffer.clone(), range);
    }
    Ok((kept, resident))
}

/// The districts: their collision is streamed in windows around the skater.
pub const DISTRICTS: [&str; 3] = ["Industrial", "University", "DownTown"];
/// The streamed collision window: half side and rebuild distance in metres (collision
/// bit-identical to the whole world from a 56 m margin). 60 / 8 since the retail audio (was
/// 100 / 25): windows of up to 187k triangles (23 MB) did not fit the heap beside it (80 m still
/// reached 83k in DownTown); the next build starts 8 m out, 12 m before the skater would wait.
pub const WINDOW: (f32, f32) = (60.0, 8.0);
/// Streamed-world buffer sets reserved at boot and their size in triangles (80 m windows held
/// 83k at most in DownTown; 60 m about 0.56 of that).
pub const WINDOW_BUFFERS: (usize, usize) = (3, 64_000);

/// What the renderer needs of the open district: the file (for tiles and textures read by offset)
/// and its tile table. Set by `load_district` on the simulation thread, taken by the main thread.
pub struct DistrictView {
    pub name: String,
    pub file: std::sync::Arc<vita_pack::district::DistrictFile>,
    pub tiles: std::collections::HashMap<(i32, i32), vita_pack::district::Tile>,
    pub textures: Vec<(u64, u32)>,
    pub tile_size: f32,
    pub horizon: [f32; 3],
    pub exposure: f32,
}

/// The district of the loaded session (None for a park or the Course) and a counter that changes
/// with every load, so that the main thread notices.
pub static DISTRICT: std::sync::Mutex<Option<std::sync::Arc<DistrictView>>> = std::sync::Mutex::new(None);
pub static DISTRICT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn clear_district() {
    set_district(None);
}

fn set_district(view: Option<std::sync::Arc<DistrictView>>) {
    *DISTRICT.lock().unwrap() = view;
    DISTRICT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// A district: its pack is opened by its index only; the collision windows read the clusters
/// they need from the file, and the render reads tiles the same way. The pack is
/// checked in the background (a mismatch sets [`DAMAGED`]).
fn load_district(name: &str) -> Result<(skate_data::skate_map::SkateMap, Render), String> {
    let path = std::path::PathBuf::from(format!("{DIR}/district_{name}.pak"));
    let started = std::time::Instant::now();
    let (district, file) = vita_pack::district::open(&path)?;
    println!("district {name}: index of {} clusters read in {:.2} s", district.clusters.len(), started.elapsed().as_secs_f32());
    // The collision windows read on a handle of their own (they used to queue behind the tile
    // streaming's reads on one lock; 0.2-4.7 s per window read in DownTown).
    let collision = std::sync::Arc::new(file.reopen()?);
    let file = std::sync::Arc::new(file);
    skate_sim::collision_window::install_district(skate_sim::collision_window::DistrictCollision {
        clusters: district.clusters.clone(),
        source: collision,
        limits: district.limits,
    });
    let check = path.clone();
    let _ = std::thread::Builder::new().name("district-verify".into()).spawn(move || {
        let _ = vita_platform::vita::pin_current_thread(2);
        // Below the window builder (170): hashing 250 MB must not starve a collision window.
        vita_platform::vita::set_current_thread_priority(190);
        let started = std::time::Instant::now();
        match vita_pack::district::verify_file(&check) {
            Ok(()) => println!("{} verified in {:.1} s", check.display(), started.elapsed().as_secs_f32()),
            Err(e) => {
                println!("{e}");
                *DAMAGED.lock().unwrap() = Some("a district pack is damaged: upload it again".into());
            }
        }
    });
    let e = &district.sim.environment;
    let horizon = [e.get(3).copied().unwrap_or(0.3), e.get(4).copied().unwrap_or(0.4), e.get(5).copied().unwrap_or(0.5)];
    set_district(Some(std::sync::Arc::new(DistrictView {
        name: name.to_owned(),
        file,
        tiles: district.tiles.iter().map(|t| ((t.cx, t.cz), *t)).collect(),
        textures: district.textures.clone(),
        tile_size: district.tile_size,
        horizon,
        exposure: 1.4,
    })));
    let render = Render {
        vertex_data: Vec::new(),
        indices: Vec::new(),
        draws: Vec::new(),
        textures: Vec::new(),
        horizon: [e.get(3).copied().unwrap_or(0.3), e.get(4).copied().unwrap_or(0.4), e.get(5).copied().unwrap_or(0.5)],
        exposure: 1.4,
    };
    Ok((district.sim, render))
}

/// The park's pack: (sim map for the session, render data for the scene).
pub fn load_park(name: &str) -> Result<(skate_data::skate_map::SkateMap, Render), String> {
    // Sessions loaded afterwards stream their collision for a district and hold it whole
    // for a park.
    if DISTRICTS.contains(&name) {
        skate_sim::collision_window::configure(WINDOW.0, WINDOW.1);
        // Windows are built on core 0 or 2, whichever is idle, while the simulation keeps
        // ticking on core 1. Core 2 alone (with the retail audio render on it) made builds take
        // 2.5–5.6 s and stalled the simulation 2.9 s waiting for one. Just below the game
        // threads' priority, so a build only takes idle time.
        skate_sim::collision_window::configure_background(Some(|| {
            crate::alloc::tag_thread("window");
            let _ = vita_platform::vita::allow_current_thread_on(&[0, 2]);
            vita_platform::vita::set_current_thread_priority(170);
        }));
    } else {
        skate_sim::collision_window::configure(0.0, 0.0);
    }
    set_district(None);
    if DISTRICTS.contains(&name) && std::path::Path::new(&format!("{DIR}/district_{name}.pak")).is_file() {
        return load_district(name);
    }
    let path = format!("{DIR}/map_{name}.pak");
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let map = Map::from_pack(&bytes)?;
    Ok((map.sim, map.render))
}
