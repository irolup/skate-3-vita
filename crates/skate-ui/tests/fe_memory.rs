//! Diagnostic: the live heap bytes of the front end's movie sets (a counting allocator in this test binary).
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
static ALLOCS: AtomicIsize = AtomicIsize::new(0);
static LARGE: AtomicIsize = AtomicIsize::new(0);
static LARGE_BYTES: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed) + layout.size() as isize;
        PEAK.fetch_max(live, Ordering::Relaxed);
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        if layout.size() > 4096 {
            LARGE.fetch_add(1, Ordering::Relaxed);
            LARGE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn measure<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let out = f();
    let (live, peak) = (LIVE.load(Ordering::Relaxed) - base, PEAK.load(Ordering::Relaxed) - base);
    eprintln!("FE_MEMORY {label}: live {} KiB, peak {} KiB", live / 1024, peak / 1024);
    out
}

#[test]
#[ignore = "diagnostic; requires the exported set (SKATE3_MENU_MOVIES=<set>)"]
fn menu_set_live_bytes() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
    let read = |path: &str| std::fs::read(root.join(path)).map_err(|e| e.to_string());
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;
    let all = measure("lean, all 39 movies", || RetailMenuMovies::read_lean(&read, None).unwrap());
    drop(all);
    let roots = ["source/screens/main/FE_root", "source/helper/screenmanager", "source/screens/main/core_menu", "source/screens/options/options", "source/screens/options/videocal", "source/screens/popup/sk8popup", "source/screens/popup/small_popup"];
    let pause = measure("lean, the pause menu's screens", || RetailMenuMovies::read_lean(&read, Some(&roots)).unwrap());
    eprintln!("  {} movies", pause.movies.len());
    drop(pause);
    let core = measure("lean, core_menu alone", || RetailMenuMovies::read_lean(&read, Some(&["source/screens/main/core_menu"])).unwrap());
    eprintln!("  {} movies", core.movies.len());
}

/// The pause front end as the Vita would hold it: the `PAUSE_ROOTS` set booted, then with the pause menu open, then
/// closed again. `SKATE3_MAIN_CHECKOUT=<install>` gives the menu tables and input.cfg.
#[test]
#[ignore = "diagnostic; requires the exported set (SKATE3_MENU_MOVIES=<set>, SKATE3_MAIN_CHECKOUT=<install>)"]
fn pause_stage_live_bytes() {
    use skate_ui::fe::menu_stage::{MenuStage, APT_START, PAUSE_ROOTS};
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
    let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_MAIN_CHECKOUT").unwrap()).join("assets");
    let read_set = |path: &str| std::fs::read(root.join(path)).map_err(|e| e.to_string());
    let read = |path: &str| std::fs::read(assets.join(path)).map_err(|e| e.to_string());
    let base = LIVE.load(Ordering::Relaxed);
    let movies = measure("lean, PAUSE_ROOTS", || RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap());
    eprintln!("  {} movies", movies.movies.len());
    let mut stage = measure("  + boot", || MenuStage::boot(movies, &read).unwrap());
    measure("  + pause menu open", || {
        stage.update_keys(vec![(APT_START, 0)], 1.0 / 60.0);
        for _ in 0..90 {
            stage.update_keys(Vec::new(), 1.0 / 60.0);
        }
        assert!(stage.open());
        stage.draw()
    });
    let peak = PEAK.load(Ordering::Relaxed) - base;
    measure("  + closed", || {
        stage.update_keys(vec![(APT_START, 0)], 1.0 / 60.0);
        for _ in 0..90 {
            stage.update_keys(Vec::new(), 1.0 / 60.0);
        }
        assert!(!stage.open());
    });
    eprintln!("FE_MEMORY total live {} KiB, peak while open {} KiB", (LIVE.load(Ordering::Relaxed) - base) / 1024, peak / 1024);
}

#[test]
#[ignore = "diagnostic; requires the exported set (SKATE3_MENU_MOVIES=<set>)"]
fn one_movie_breakdown() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
    let shared_json: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("shared.json")).unwrap()).unwrap();
    let shared = measure("shared text", || skate_ui::fe::apt_text::SharedText::from_json(&shared_json).unwrap());
    for key in ["source/screens/main/core_menu", "source/screens/options/options", "source/helper/screenmanager"] {
        let bytes = std::fs::read(root.join(format!("movies/{key}.json"))).unwrap();
        eprintln!("{key}: {} KiB of JSON", bytes.len() / 1024);
        let json: serde_json::Value = measure("  parsed JSON", || serde_json::from_slice(&bytes).unwrap());
        let shapes: skate_ui::fe::apt_scene::Shapes = measure("  shapes", || serde_json::from_value(json["shapes"].clone()).unwrap());
        let movie = measure("  movie", || skate_ui::fe::apt_movie::Movie::load_shared(&json, &shared).unwrap());
        drop((shapes, movie));
    }
}

#[test]
#[ignore = "diagnostic"]
fn boot_pieces() {
    let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_MAIN_CHECKOUT").unwrap()).join("assets");
    let text = std::fs::read_to_string(assets.join("private/menu-tables.json")).unwrap();
    let tables = measure("tables parse", || skate_data::menu_tables::MenuTables::from_json(&text).unwrap());
    let data = measure("to_menu_data", || tables.to_menu_data());
    let menus = measure("RetailMenus::new", || skate_ui::fe::retail_menus::RetailMenus::new(data, true));
    let bytes = std::fs::read(assets.join("private/audio/audio_manifest.json")).unwrap();
    let manifest: serde_json::Value = measure("audio manifest", || serde_json::from_slice(&bytes).unwrap());
    drop((manifest, menus));
}

#[test]
#[ignore = "diagnostic"]
fn movie_field_sizes() {
    use skate_ui::fe::menu_stage::PAUSE_ROOTS;
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
    let read = |path: &str| std::fs::read(root.join(path)).map_err(|e| e.to_string());
    let set = RetailMenuMovies::read_lean(&read, Some(&PAUSE_ROOTS)).unwrap();
    let mut total = [0isize; 6];
    for (key, m) in &set.movies {
        let size = |f: &dyn Fn() -> Box<dyn std::any::Any>| {
            let base = LIVE.load(Ordering::Relaxed);
            let v = f();
            let n = LIVE.load(Ordering::Relaxed) - base;
            drop(v);
            n
        };
        let whole = size(&|| Box::new(m.clone()));
        let chars = size(&|| Box::new((*m.characters).clone()));
        let actions = size(&|| Box::new((*m.actions).clone()));
        let instances = size(&|| Box::new(m.instances.clone()));
        let text = size(&|| Box::new(m.text_assets.clone()));
        let rest = whole - chars - actions - instances - text;
        for (t, v) in total.iter_mut().zip([whole, chars, actions, instances, text, rest]) {
            *t += v;
        }
        eprintln!("{key}: clone {} KiB = characters {} + actions {} + instances {} + text {} + other {}", whole / 1024, chars / 1024, actions / 1024, instances / 1024, text / 1024, rest / 1024);
    }
    eprintln!("TOTAL KiB {:?} (clone, characters, actions, instances, text, other)", total.map(|v| v / 1024));
}

#[test]
#[ignore = "diagnostic"]
fn shared_text_breakdown() {
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
    let shared_json: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("shared.json")).unwrap()).unwrap();
    let language: std::collections::BTreeMap<String, String> = measure("language table", || serde_json::from_value(shared_json["language"].clone()).unwrap());
    let fonts = measure("fonts JSON tree", || shared_json["fonts"].clone());
    drop((language, fonts));
    measure("SharedText", || skate_ui::fe::apt_text::SharedText::from_json(&shared_json).unwrap());
}

/// Frame cost of the open pause menu (no keys): `stage.update` and `stage.draw` per frame.
#[test]
#[ignore = "diagnostic; requires the exported set (SKATE3_MENU_MOVIES=<set>, SKATE3_MAIN_CHECKOUT=<install>)"]
fn pause_stage_frame_cost() {
    use skate_ui::fe::menu_stage::{MenuStage, APT_START, PAUSE_ROOTS};
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
    let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_MAIN_CHECKOUT").unwrap()).join("assets");
    let read_set = |path: &str| std::fs::read(root.join(path)).map_err(|e| e.to_string());
    let read = |path: &str| std::fs::read(assets.join(path)).map_err(|e| e.to_string());
    let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
    let mut stage = MenuStage::boot(movies, &read).unwrap();
    skate_ui::fe::profile::set_allocation_counter(|| ALLOCS.load(Ordering::Relaxed) as u64);
    let t = std::time::Instant::now();
    let a = ALLOCS.load(Ordering::Relaxed);
    stage.update_keys(vec![(APT_START, 0)], 1.0 / 60.0);
    for _ in 0..30 {
        stage.update_keys(Vec::new(), 1.0 / 60.0);
    }
    eprintln!("FE_COST open (31 frames) {:.2} ms, {} allocations", t.elapsed().as_secs_f64() * 1e3, ALLOCS.load(Ordering::Relaxed) - a);
    for (name, ms, count, allocs) in skate_ui::fe::profile::take() {
        eprintln!("FE_COST open span {name}: {ms:.3} ms, {allocs} allocations ({count} entries)");
    }
    let frames: usize = std::env::var("FE_FRAMES").ok().and_then(|f| f.parse().ok()).unwrap_or(600);
    let (mut update, mut draw, mut worst) = (0.0, 0.0, 0.0f64);
    for i in 0..frames {
        let t = std::time::Instant::now();
        stage.update_keys(if i % 60 == 59 { vec![(15, 0)] } else { Vec::new() }, 1.0 / 60.0);
        let u = t.elapsed().as_secs_f64() * 1e3;
        let t = std::time::Instant::now();
        let n = stage.draw().len();
        draw += t.elapsed().as_secs_f64() * 1e3;
        update += u;
        worst = worst.max(u);
        if i == frames - 1 {
            eprintln!("FE_COST {n} draws");
        }
    }
    let n = frames as f64;
    for (name, ms, count, allocs) in skate_ui::fe::profile::take() {
        eprintln!("FE_COST span {name}: {:.4} ms, {:.0} allocations per frame ({count} entries)", ms / n, allocs as f64 / n);
    }
    let (a0, l0, b0) = (ALLOCS.load(Ordering::Relaxed), LARGE.load(Ordering::Relaxed), LARGE_BYTES.load(Ordering::Relaxed));
    for _ in 0..60 {
        stage.update_keys(Vec::new(), 1.0 / 60.0);
    }
    eprintln!(
        "FE_COST per idle update: {} allocations, {} over 4 KiB ({} KiB)",
        (ALLOCS.load(Ordering::Relaxed) - a0) / 60,
        (LARGE.load(Ordering::Relaxed) - l0) / 60,
        (LARGE_BYTES.load(Ordering::Relaxed) - b0) / 60 / 1024
    );
    eprintln!("FE_COST update {:.3} ms (worst {:.2}), draw {:.3} ms per frame over {frames} frames", update / n, worst, draw / n);
}

/// A hash of every frame's draws over a scripted walk (open, tabs, rows, Game Settings, back, close): the runtime's
/// performance changes must leave it unchanged.
#[test]
#[ignore = "diagnostic; requires the exported set (SKATE3_MENU_MOVIES=<set>, SKATE3_MAIN_CHECKOUT=<install>)"]
fn pause_stage_draw_hash() {
    use skate_ui::fe::menu_stage::{MenuStage, APT_START, PAUSE_ROOTS};
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;
    use std::hash::{Hash, Hasher};
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
    let assets = std::path::PathBuf::from(std::env::var_os("SKATE3_MAIN_CHECKOUT").unwrap()).join("assets");
    let read_set = |path: &str| std::fs::read(root.join(path)).map_err(|e| e.to_string());
    let read = |path: &str| std::fs::read(assets.join(path)).map_err(|e| e.to_string());
    let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
    let mut stage = MenuStage::boot(movies, &read).unwrap();
    // Keys by frame: Start, right x3, down x2, left, Game Settings is reached by right/A probes, back, Start.
    // Keys by frame: Start, Right x3 to Options, A on Game Settings, Down x2, Right / Left (step a row), B (back to the
    // pause menu), Right, Start (close).
    let script: Vec<(usize, u32)> = vec![
        (5, APT_START), (90, 15), (110, 15), (130, 15), (200, 2), (330, 13), (350, 13), (370, 15), (390, 14), (420, 13),
        (440, 15), (500, 3), (700, 15), (720, 13), (900, APT_START),
    ];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut log = Vec::new();
    for frame in 0..1200 {
        let keys: Vec<(u32, usize)> = script.iter().filter(|(f, _)| *f == frame).map(|(_, k)| (*k, 0)).collect();
        let out = stage.update_keys(keys, 1.0 / 60.0);
        log.extend(out.log.iter().filter(|l| !l.starts_with("APT objects")).cloned());
        for d in stage.draw() {
            d.texture.hash(&mut hasher);
            for v in &d.vertices {
                v.position.map(f32::to_bits).hash(&mut hasher);
                v.uv.map(f32::to_bits).hash(&mut hasher);
            }
            d.multiply.map(f32::to_bits).hash(&mut hasher);
            d.add.map(f32::to_bits).hash(&mut hasher);
        }
        frame.hash(&mut hasher);
    }
    log.hash(&mut hasher);
    eprintln!("FE_HASH {:016x} ({} log lines: {:?})", hasher.finish(), log.len(), log.iter().filter(|l| !l.starts_with("key")).collect::<Vec<_>>());
}
