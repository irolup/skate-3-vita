//! Memory hunt (feature `heap-probe`): a counting global allocator and a long scripted
//! session on the baked packs, to find what keeps growing on the Vita's heap.
//!
//! `sim-oracle heap-probe --packs DIR --map NAME TICKS`: the session loads as the Vita loads it
//! (packs in the virtual file system, a district's collision streamed in windows), starts the
//! living world (`PROBE_PEDS=1` turns the peds on), and rides a scripted line (push, carve left
//! and right) for TICKS ticks. Every 1000 ticks it prints the live bytes in small (up to 4 KiB, as
//! the Vita's allocator splits them) and large blocks; allocations are sampled by bytes (one per
//! 64 KiB allocated, every one of 64 KiB or more) and keep their call site while they live; at
//! the end the sites holding the most memory, and those that grew most since tick 3000, are listed.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static SMALL: AtomicUsize = AtomicUsize::new(0);
static LARGE: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
/// Sampling by bytes (unbiased for any size): an allocation of at least `PERIOD` bytes is always
/// recorded with its size; a smaller one when the running byte count crosses a multiple of
/// `PERIOD`, standing for `PERIOD` bytes.
const PERIOD: u64 = 64 * 1024;

thread_local! { static INSIDE: Cell<bool> = const { Cell::new(false) }; }

/// Live sampled allocations: address -> (site, weight in bytes).
static LIVE: Mutex<Option<HashMap<usize, (u32, usize)>>> = Mutex::new(None);
/// Site names by id.
static SITES: Mutex<Option<(HashMap<String, u32>, Vec<String>)>> = Mutex::new(None);

fn site_of() -> String {
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    let frames: Vec<&str> = bt
        .lines()
        .map(str::trim)
        .filter(|l| (l.contains("skate_") || l.contains("vita_pack") || l.contains("serde_json")) && !l.contains("heap_probe") && !l.starts_with("at "))
        .map(|l| l.split_once(": ").map_or(l, |x| x.1))
        .take(4)
        .collect();
    if frames.is_empty() { "?".into() } else { frames.join(" <- ") }
}

fn sampled(size: usize) -> Option<usize> {
    let size = size as u64;
    if size >= PERIOD {
        return Some(size as usize);
    }
    let before = BYTES.fetch_add(size, Ordering::Relaxed);
    (before / PERIOD != (before + size) / PERIOD).then_some(PERIOD as usize)
}

fn record(p: *mut u8, size: usize) {
    if INSIDE.with(|i| i.replace(true)) {
        return;
    }
    let site = site_of();
    if size >= 4 << 20 {
        eprintln!("BIG {:.2} MB: {site}", size as f64 / 1e6);
    }
    let id = {
        let mut s = SITES.lock().unwrap();
        let (map, names) = s.get_or_insert_with(Default::default);
        *map.entry(site.clone()).or_insert_with(|| {
            names.push(site);
            (names.len() - 1) as u32
        })
    };
    LIVE.lock().unwrap().get_or_insert_with(HashMap::new).insert(p as usize, (id, size));
    INSIDE.with(|i| i.set(false));
}

fn forget(p: *mut u8) {
    if INSIDE.with(|i| i.replace(true)) {
        return;
    }
    if let Some(m) = LIVE.lock().unwrap().as_mut() {
        m.remove(&(p as usize));
    }
    INSIDE.with(|i| i.set(false));
}

fn counter(size: usize) -> &'static AtomicUsize {
    if size <= 4096 { &SMALL } else { &LARGE }
}

pub struct Probe;

unsafe impl GlobalAlloc for Probe {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            counter(l.size()).fetch_add(l.size(), Ordering::Relaxed);
            if let Some(weight) = sampled(l.size()) {
                record(p, weight);
            }
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        counter(l.size()).fetch_sub(l.size(), Ordering::Relaxed);
        forget(p);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            counter(l.size()).fetch_sub(l.size(), Ordering::Relaxed);
            counter(n).fetch_add(n, Ordering::Relaxed);
            forget(p);
            if let Some(weight) = sampled(n) {
                record(q, weight);
            }
        }
        q
    }
}

#[global_allocator]
static GLOBAL: Probe = Probe;

/// Sampled live bytes per site id.
fn by_site() -> HashMap<u32, usize> {
    let mut out = HashMap::new();
    INSIDE.with(|i| i.set(true));
    if let Some(m) = LIVE.lock().unwrap().as_ref() {
        for (id, size) in m.values() {
            *out.entry(*id).or_insert(0) += size;
        }
    }
    INSIDE.with(|i| i.set(false));
    out
}

fn name(id: u32) -> String {
    INSIDE.with(|i| i.set(true));
    let n = SITES.lock().unwrap().as_ref().and_then(|(_, names)| names.get(id as usize).cloned()).unwrap_or_default();
    INSIDE.with(|i| i.set(false));
    n
}

fn mb(b: usize) -> f64 {
    b as f64 / 1e6
}

pub fn run(packs: &Path, map_name: &str, ticks: u64) -> Result<bool, String> {
    let root = crate::mount_packs(packs)?;
    if let Ok(bytes) = std::fs::read(packs.join("living.pak")) {
        for (path, data) in vita_pack::files::from_pack(&bytes)? {
            skate_data::vfs::mount(Path::new(&root).join(path), data);
        }
        eprintln!("mounted {}/living.pak", packs.display());
    }
    // The device's streamed collision (packs::WINDOW: 60 m half, 8 m trigger).
    skate_sim::collision_window::configure(60.0, 8.0);
    let district = packs.join(format!("district_{map_name}.pak"));
    let (district, file) = vita_pack::district::open(&district)?;
    skate_sim::collision_window::install_district(skate_sim::collision_window::DistrictCollision {
        clusters: district.clusters.clone(),
        source: std::sync::Arc::new(file),
        limits: district.limits,
    });
    let loaded = (SMALL.load(Ordering::Relaxed), LARGE.load(Ordering::Relaxed));
    let mut session = skate_sim::live::Session::load_map(Path::new(&root), "normal", Some(district.sim))?;
    skate_sim::live::enable_game_switches();
    let mut settings = skate_sim::LivingWorldSettings::default();
    settings.vehicles.density = 0.4;
    settings.pedestrians.enabled = std::env::var_os("PROBE_PEDS").is_some();
    settings.pedestrians.density = 0.25;
    let living = session.start_living_world(settings);
    session.set_animation_budget(16 << 20);
    eprintln!(
        "loaded {map_name} (living world {living}): small {:.1} MB, large {:.1} MB (before the session: {:.1} / {:.1})",
        mb(SMALL.load(Ordering::Relaxed)),
        mb(LARGE.load(Ordering::Relaxed)),
        mb(loaded.0),
        mb(loaded.1)
    );
    let mut baseline: Option<HashMap<u32, usize>> = None;
    for t in 0..ticks {
        // Push for 1.5 s of every 4, carve one way then the other every 6 s.
        let push = t % 240 < 90;
        let carve = ((t / 360) % 2) as i16 * 2 - 1;
        let pad = skate_core::input::xbox::XboxState { buttons: if push { 0x1000 } else { 0 }, triggers: [0; 2], left: [carve * 14000, 0], right: [0; 2] };
        let report = session.step(pad);
        if let Some(e) = report.error {
            eprintln!("tick {t}: {e}; respawn");
            let _ = session.respawn();
        }
        if t % 1800 == 1799 {
            // Back to the spawn now and then, so the ride covers the same ground and new ground.
            let _ = session.respawn();
        }
        if t == 3000 {
            baseline = Some(by_site());
        }
        if t % 1000 == 999 {
            let counts = session.living_counts().unwrap_or_default();
            println!(
                "tick {:>6}: small {:>6.1} MB, large {:>6.1} MB, decoded animation {:.1} MB; living world {:?}",
                t + 1,
                mb(SMALL.load(Ordering::Relaxed)),
                mb(LARGE.load(Ordering::Relaxed)),
                mb(session.animation_bytes()),
                counts
            );
        }
    }
    let now = by_site();
    let mut top: Vec<(u32, usize)> = now.iter().map(|(k, v)| (*k, *v)).collect();
    top.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("\nlargest live sites (byte-sampled, MB):");
    for (id, bytes) in top.iter().take(25) {
        println!("  {:>7.2} MB  {}", mb(*bytes), name(*id));
    }
    if let Some(base) = baseline {
        let mut grew: Vec<(u32, i64)> = now.iter().map(|(k, v)| (*k, *v as i64 - *base.get(k).unwrap_or(&0) as i64)).collect();
        for (k, v) in &base {
            if !now.contains_key(k) {
                grew.push((*k, -(*v as i64)));
            }
        }
        grew.sort_by_key(|x| std::cmp::Reverse(x.1));
        println!("\ngrowth since tick 3000 (byte-sampled, MB):");
        for (id, bytes) in grew.iter().take(25) {
            println!("  {:>+7.2} MB  {}", *bytes as f64 / 1e6, name(*id));
        }
    }
    Ok(true)
}

/// `sim-oracle heap-probe-ui --packs DIR`: the retail HUD and menu movies as the Vita loads them
/// (ui.pak in memory), with the live bytes after each step (64-bit sizes: the Vita's are smaller
/// for pointer-heavy data).
pub fn run_ui(packs: &Path) -> Result<bool, String> {
    let root = crate::mount_packs(packs)?;
    let before = SMALL.load(Ordering::Relaxed) + LARGE.load(Ordering::Relaxed);
    let collections = skate_data::collections::Collections::load(Path::new(&root))?;
    println!(
        "skater collections loaded: {:+.1} MB ({} entries)",
        (SMALL.load(Ordering::Relaxed) + LARGE.load(Ordering::Relaxed) - before) as f64 / 1e6,
        collections.entries().len()
    );
    let bytes = std::fs::read(packs.join("ui.pak")).map_err(|e| e.to_string())?;
    let files: HashMap<String, std::sync::Arc<[u8]>> = vita_pack::files::from_pack(&bytes)?.into_iter().collect();
    drop(bytes);
    let file = |p: &str| files.get(p).ok_or(format!("{p}: not in ui.pak"));
    let live = || SMALL.load(Ordering::Relaxed) + LARGE.load(Ordering::Relaxed);
    let step = |what: &str, since: usize| {
        let now = live();
        println!("{what}: {:+.1} MB (small {:.1} MB, large {:.1} MB live)", (now as f64 - since as f64) / 1e6, mb(SMALL.load(Ordering::Relaxed)), mb(LARGE.load(Ordering::Relaxed)));
        now
    };
    let t = live();
    let hud: serde_json::Value = serde_json::from_slice(file("private/hud/runtime/trickdisplay.json")?).map_err(|e| e.to_string())?;
    let t = step("HUD trickdisplay.json parsed (kept by the Vita HUD for rebuilds)", t);
    let input = skate_ui::hud_runtime::Input {
        sequence_score: 0,
        line_score: 0,
        sequence_timer: 0,
        line_time: 0.0,
        line_capacity: 0.0,
        multiplier: 1.0,
        clean: false,
        sketchy: false,
        stance: [false; 4],
        trick_name: String::new(),
        trick_metrics: std::array::from_fn(|_| skate_ui::apt_vm::Value::Bool(false)),
        context_tricks: Vec::new(),
    };
    let runtime = skate_ui::hud_runtime::Runtime::load(&hud, input)?;
    let t = step("HUD runtime built", t);
    let shapes: serde_json::Value = hud["shapes"].clone();
    let t = step("HUD shapes (as JSON; the Vita keeps them as Shapes)", t);
    drop(shapes);
    let menu: serde_json::Value = serde_json::from_slice(file("private/ui/core_menu.linked.json")?).map_err(|e| e.to_string())?;
    let t = step("menu core_menu.linked.json parsed (dropped after the build)", t);
    let crossbar = skate_ui::crossbar::Crossbar::load(&menu, Vec::new())?;
    let t = step("menu crossbar built", t);
    drop(menu);
    let t = step("menu JSON dropped", t);
    drop(runtime);
    let t = step("HUD runtime dropped", t);
    drop(hud);
    let t = step("HUD JSON dropped", t);
    drop(crossbar);
    step("crossbar dropped", t);
    Ok(true)
}
