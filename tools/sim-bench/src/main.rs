//! Headless simulation benchmark and on-device parity oracle.
//!
//! Replays every `ux0:data/SKATEVITA/scenarios/*.input` through the shared oracle harness
//! (production load from `ux0:data/SKATEVITA/assets`, procedural Course), writes dumps to
//! `ux0:data/SKATEVITA/oracle/`, and prints per-tick timings. Clocks are set to
//! 444/222/222/166 MHz and the simulation runs on its own thread on core 1. Every thread
//! runs in IEEE float mode (tools/vita-fpu). scripts/vita-bench.py drives it.
//!
//! Nothing is drawn: the screen stays black while it runs (the idle timers are reset, so
//! the Vita does not enter standby). Press Start to cancel; the
//! process always exits back to LiveArea when it finishes, fails or is cancelled.
//!
//! On desktop it runs the same code on local folders:
//! `sim-bench ASSETS SCENARIOS OUT`.

#[cfg(target_os = "vita")]
use vita_newlib_shims as _;

#[cfg(target_os = "vita")]
#[used]
#[unsafe(export_name = "sceUserMainThreadStackSize")]
pub static STACK: u32 = 2 * 1024 * 1024;
#[cfg(target_os = "vita")]
#[used]
#[unsafe(export_name = "_newlib_heap_size_user")]
pub static HEAP: u32 = 224 * 1024 * 1024;
#[cfg(target_os = "vita")]
#[used]
#[unsafe(export_name = "sceLibcHeapSize")]
pub static LIBC: u32 = 8 * 1024 * 1024;

mod profile;

#[cfg(feature = "alloc-count")]
mod alloc_count {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering};
    pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
    pub static BYTES: AtomicU64 = AtomicU64::new(0);
    pub struct Counting;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::Mutex;
    thread_local! { static INSIDE: Cell<bool> = const { Cell::new(false) }; }
    /// Sampled call sites of allocations made inside `frame::advance`: (site, count).
    pub static SITES: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
    fn sample() {
        if INSIDE.with(|i| i.replace(true)) {
            return;
        }
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let in_tick = bt.contains("frame7advance") || bt.contains("frame::advance");
        let want_load = std::env::var_os("ALLOC_SITES").is_some_and(|v| v == "load");
        if in_tick != want_load {
            // First skate_core / skate_sim frame that is not the allocator plumbing.
            let site = bt
                .lines()
                .map(str::trim)
                .filter(|l| l.contains("skate_core::") || l.contains("skate_sim::"))
                .find(|l| !l.contains("alloc_count"))
                .unwrap_or("?")
                .to_string();
            let mut sites = SITES.lock().unwrap();
            *sites.get_or_insert_with(HashMap::new).entry(site).or_insert(0) += 1;
        }
        INSIDE.with(|i| i.set(false));
    }
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let n = ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
            if n % 64 == 0 && std::env::var_os("ALLOC_SITES").is_some() {
                sample();
            }
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(n as u64, Ordering::Relaxed);
            unsafe { System.realloc(p, l, n) }
        }
    }
    #[global_allocator]
    static GLOBAL: Counting = Counting;
}

use std::path::{Path, PathBuf};

#[cfg(all(feature = "mem-count", target_os = "vita"))]
mod mem_count {
    unsafe extern "C" {
        static mut skate_vita_aeabi_calls: [[u64; 4]; 3];
        static mut skate_vita_aeabi_bytes: [[u64; 4]; 3];
        static mut skate_vita_aeabi_sites: [Site; 1024];
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Site {
        addr: u32,
        kind: u32,
        calls: u64,
        bytes: u64,
    }
    pub fn reset() {
        unsafe {
            std::ptr::write(&raw mut skate_vita_aeabi_calls, [[0; 4]; 3]);
            std::ptr::write(&raw mut skate_vita_aeabi_bytes, [[0; 4]; 3]);
            std::ptr::write_bytes(&raw mut skate_vita_aeabi_sites, 0, 1);
        }
    }
    /// Per-tick calls/bytes of the AEABI memory builtins (load phase included in the
    /// first scenario); resets.
    pub fn report(name: &str, ticks: usize) {
        let (calls, bytes) = unsafe {
            let c = std::ptr::read(&raw const skate_vita_aeabi_calls);
            let b = std::ptr::read(&raw const skate_vita_aeabi_bytes);
            std::ptr::write(&raw mut skate_vita_aeabi_calls, [[0; 4]; 3]);
            std::ptr::write(&raw mut skate_vita_aeabi_bytes, [[0; 4]; 3]);
            (c, b)
        };
        let t = ticks.max(1) as f64;
        let mut sites = unsafe {
            let s = std::ptr::read(&raw const skate_vita_aeabi_sites);
            std::ptr::write_bytes(&raw mut skate_vita_aeabi_sites, 0, 1);
            s
        };
        sites.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        // Runtime address of a known function, to rebase the sites onto the ELF.
        println!("BENCH MEMBASE report={:#010x}", report as fn(&str, usize) as usize);
        for s in sites.iter().take(40).filter(|s| s.addr != 0) {
            println!(
                "BENCH MEMSITE {name} {:#010x} {} calls_per_tick={:.1} bytes_per_tick={:.0}",
                s.addr,
                ["copy", "move", "set"][s.kind as usize],
                s.calls as f64 / t,
                s.bytes as f64 / t
            );
        }
        for (k, kind) in ["copy", "move", "set"].iter().enumerate() {
            for (c, class) in ["<64", "<256", "<1024", ">=1024"].iter().enumerate() {
                println!(
                    "BENCH MEM {name} {kind} {class} calls_per_tick={:.1} bytes_per_tick={:.0}",
                    calls[k][c] as f64 / t,
                    bytes[k][c] as f64 / t
                );
            }
        }
    }
}

static PROFILER: std::sync::LazyLock<profile::Profiler> = std::sync::LazyLock::new(profile::Profiler::new);

/// Per-tick stage rows (feature `tick-stages`): (stage, µs or counter value).
#[cfg(feature = "tick-stages")]
static TICK_ROWS: std::sync::Mutex<Vec<Vec<(&'static str, u64)>>> = std::sync::Mutex::new(Vec::new());

/// kernel-bench: the tick capture starts at (06's walk, 19's wipeout).
#[cfg(feature = "kernel-bench")]
static CAPTURE_TICK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1600);

#[cfg(any(feature = "tick-stages", feature = "kernel-bench", feature = "mem-count"))]
fn tick_hook(_tick: usize) {
    #[cfg(all(feature = "mem-count", target_os = "vita"))]
    if _tick == 0 {
        mem_count::reset(); // count the ticks, not the load
    }
    #[cfg(feature = "kernel-bench")]
    if _tick + 1 == CAPTURE_TICK.load(std::sync::atomic::Ordering::Relaxed) {
        skate_core::physics::world_contact::kernel_bench::set_capture(true);
    }
    #[cfg(feature = "tick-stages")]
    tick_stages_row();
}

#[cfg(feature = "tick-stages")]
fn tick_stages_row() {
    let mut row: Vec<(&'static str, u64)> =
        PROFILER.take().into_iter().map(|(n, d, _)| (n, d.as_micros() as u64)).collect();
    for (n, v, _) in skate_sim::profile_take() {
        let counter = n.starts_with("n_") || n.starts_with("q_");
        row.push((n, if counter { v } else { v / 1000 }));
    }
    TICK_ROWS.lock().unwrap().push(row);
}

#[cfg(feature = "tick-stages")]
fn write_tick_stages(path: &Path) -> Result<(), String> {
    let rows = std::mem::take(&mut *TICK_ROWS.lock().unwrap());
    let mut names: Vec<&str> = rows.iter().flatten().map(|e| e.0).collect();
    names.sort_unstable();
    names.dedup();
    let mut text = format!("tick,{}\n", names.join(","));
    for (tick, row) in rows.iter().enumerate() {
        text += &tick.to_string();
        for name in &names {
            let v: u64 = row.iter().filter(|e| e.0 == *name).map(|e| e.1).sum();
            text += &format!(",{v}");
        }
        text.push('\n');
    }
    std::fs::write(path, text).map_err(|e| e.to_string())
}

fn main() {
    let (assets, scenarios, out) = paths();
    platform_setup();
    tracing::subscriber::set_global_default(&*PROFILER).expect("install stage profiler");
    #[cfg(any(feature = "tick-stages", feature = "kernel-bench", feature = "mem-count"))]
    let _ = skate_sim::oracle_harness::TICK_HOOK.set(tick_hook);
    let handle = std::thread::Builder::new()
        .name("sim".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            pin_to_core_1();
            run(&assets, &scenarios, &out)
        })
        .expect("spawn sim thread");
    while !handle.is_finished() {
        keep_awake();
        if start_pressed() {
            println!("BENCH CANCELLED (Start)");
            exit_process();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    match handle.join() {
        Ok(Ok(())) => println!("BENCH DONE"),
        Ok(Err(e)) => println!("BENCH ERROR {e}"),
        Err(_) => println!("BENCH ERROR sim thread panicked"),
    }
    exit_process();
}

/// Resets the idle timers, so the Vita neither dims nor enters standby during a run
/// (standby suspends the process: frozen timings and a dropped Wi-Fi link).
#[cfg(target_os = "vita")]
fn keep_awake() {
    unsafe { vitasdk_sys::sceKernelPowerTick(vitasdk_sys::SCE_KERNEL_POWER_TICK_DEFAULT) };
}
#[cfg(not(target_os = "vita"))]
fn keep_awake() {}

#[cfg(target_os = "vita")]
fn start_pressed() -> bool {
    use vitasdk_sys::*;
    let mut pad: SceCtrlData = unsafe { core::mem::zeroed() };
    unsafe { sceCtrlPeekBufferPositive(0, &mut pad, 1) };
    pad.buttons & SCE_CTRL_START != 0
}
#[cfg(not(target_os = "vita"))]
fn start_pressed() -> bool {
    false
}

/// Ends the whole process, including the sim thread if it is still running.
#[cfg(target_os = "vita")]
fn exit_process() -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    // Give PrincessLog a moment to send the last line.
    std::thread::sleep(std::time::Duration::from_millis(300));
    unsafe { vitasdk_sys::sceKernelExitProcess(0) };
    std::process::exit(0)
}
#[cfg(not(target_os = "vita"))]
fn exit_process() -> ! {
    std::process::exit(0)
}

#[cfg(target_os = "vita")]
fn paths() -> (PathBuf, PathBuf, PathBuf) {
    let root = PathBuf::from("ux0:data/SKATEVITA");
    (root.join("assets"), root.join("scenarios"), root.join("oracle"))
}
#[cfg(not(target_os = "vita"))]
fn paths() -> (PathBuf, PathBuf, PathBuf) {
    let a: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    assert!(a.len() == 3, "usage: sim-bench ASSETS SCENARIOS OUT");
    (a[0].clone(), a[1].clone(), a[2].clone())
}

#[cfg(target_os = "vita")]
fn platform_setup() {
    use vitasdk_sys::*;
    unsafe {
        scePowerSetArmClockFrequency(444);
        scePowerSetBusClockFrequency(222);
        scePowerSetGpuClockFrequency(222);
        scePowerSetGpuXbarClockFrequency(166);
        println!(
            "BENCH clocks arm={} bus={} gpu={} xbar={} MHz",
            scePowerGetArmClockFrequency(),
            scePowerGetBusClockFrequency(),
            scePowerGetGpuClockFrequency(),
            scePowerGetGpuXbarClockFrequency()
        );
    }
}
#[cfg(not(target_os = "vita"))]
fn platform_setup() {}

#[cfg(target_os = "vita")]
fn pin_to_core_1() {
    use vitasdk_sys::*;
    let ret = unsafe {
        sceKernelChangeThreadCpuAffinityMask(sceKernelGetThreadId(), SCE_KERNEL_CPU_MASK_USER_1 as i32)
    };
    println!("BENCH sim thread on core 1 ({ret:#x})");
    let fpscr: u32;
    unsafe { core::arch::asm!("vmrs {}, fpscr", out(reg) fpscr) };
    println!("BENCH sim thread fpscr={fpscr:#010x}");
}
#[cfg(not(target_os = "vita"))]
fn pin_to_core_1() {}

fn percentile(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

/// `packs` feature: mounts sim.pak + anim.pak (strict) and returns the virtual root.
#[cfg(feature = "packs")]
fn mount_packs(dir: &Path) -> Result<PathBuf, String> {
    let started = std::time::Instant::now();
    let root = PathBuf::from("vfs:/assets");
    let mut read_s = 0.0;
    for name in ["sim.pak", "anim.pak"] {
        let t = std::time::Instant::now();
        let bytes = std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        read_s += t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        vita_pack::open(&bytes)?;
        println!("BENCH MOUNT {name} hash_check_s={:.2} bytes={}", t.elapsed().as_secs_f64(), bytes.len());
        for (path, data) in vita_pack::files::from_pack(&bytes)? {
            skate_data::vfs::mount(root.join(path), data);
        }
    }
    skate_data::vfs::set_strict(true);
    println!("BENCH MOUNT read_s={read_s:.2} total_s={:.2}", started.elapsed().as_secs_f64());
    Ok(root)
}

fn run(assets: &Path, scenarios: &Path, out: &Path) -> Result<(), String> {
    // The harness reads ORACLE_ITERATIONS (the device has no environment of its own).
    #[cfg(feature = "fast-physics")]
    unsafe {
        std::env::set_var("ORACLE_ITERATIONS", "25");
    }
    #[cfg(feature = "packs")]
    let packed = mount_packs(assets.parent().unwrap())?;
    #[cfg(feature = "packs")]
    let assets = packed.as_path();
    println!(
        "BENCH arithmetic {}",
        skate_sim::arithmetic()
    );
    let started = std::time::Instant::now();
    // Scenario by scenario, so the stage profile can be taken per scenario.
    let mut results = Vec::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(scenarios)
        .map_err(|e| format!("{}: {e}", scenarios.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "input"))
        .collect();
    names.sort();
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for path in names {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let inputs = skate_sim::oracle_harness::read_inputs(&path)?;
        #[cfg(feature = "kernel-bench")]
        CAPTURE_TICK.store(if name.starts_with("19_") { 690 } else { 1600 }, std::sync::atomic::Ordering::Relaxed);
        let file = std::fs::File::create(out.join(format!("{name}.dump"))).map_err(|e| e.to_string())?;
        let mut w = std::io::BufWriter::new(file);
        let _ = PROFILER.take(); // drop the load phase
        let _ = skate_sim::profile_take();
        #[cfg(feature = "tick-stages")]
        TICK_ROWS.lock().unwrap().clear();
        let load = std::time::Instant::now();
        let summary = skate_sim::oracle_harness::run(assets, &inputs, &mut w)?;
        #[cfg(feature = "alloc-count")]
        println!(
            "BENCH ALLOCS {name} total={} bytes={}",
            alloc_count::ALLOCS.swap(0, std::sync::atomic::Ordering::Relaxed),
            alloc_count::BYTES.swap(0, std::sync::atomic::Ordering::Relaxed)
        );
        #[cfg(feature = "alloc-count")]
        if let Some(sites) = alloc_count::SITES.lock().unwrap().take() {
            let mut v: Vec<_> = sites.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            for (site, n) in v.iter().take(25) {
                println!("BENCH ALLOCSITE {name} {n:>5} {site}");
            }
        }
        std::io::Write::flush(&mut w).map_err(|e| e.to_string())?;
        #[cfg(feature = "tick-stages")]
        write_tick_stages(&out.join(format!("{name}.stages")))?;
        // Per-tick wall times (u32 LE microseconds) for scripts/tick-replay.py.
        let ticks: Vec<u8> = summary.tick_us.iter().flat_map(|us| us.to_le_bytes()).collect();
        std::fs::write(out.join(format!("{name}.ticks")), ticks).map_err(|e| e.to_string())?;
        println!("BENCH {name} load+run_s={:.1}", load.elapsed().as_secs_f64());
        let steps: Vec<String> = summary.load_s.iter().map(|(n, s)| format!("{n}={s:.2}")).collect();
        println!("BENCH LOAD {name} {}", steps.join(" "));
        let ticks = summary.ticks.max(1) as f64;
        for (stage, time, count) in PROFILER.take() {
            println!(
                "BENCH STAGE {name} {stage} mean_us_per_tick={:.0} entries={count}",
                time.as_secs_f64() * 1e6 / ticks
            );
        }
        for (stage, total, count) in skate_sim::profile_take() {
            if stage.starts_with("n_") || stage.starts_with("q_") {
                println!("BENCH COUNT {name} {stage} mean={:.1}", total as f64 / count.max(1) as f64);
            } else {
                println!(
                    "BENCH STAGE {name} {stage} mean_us_per_tick={:.0} entries={count}",
                    total as f64 / 1e3 / ticks
                );
            }
        }
        #[cfg(all(feature = "mem-count", target_os = "vita"))]
        mem_count::report(&name, summary.ticks);
        #[cfg(feature = "kernel-bench")]
        for (label, calls, ns) in skate_core::physics::world_contact::kernel_bench::run(17) {
            println!("BENCH MICRO {name} {label} calls={calls} ns_per_call={ns:.0}");
        }
        results.push((name, summary));
    }
    let mut all = Vec::new();
    let mut all_advance = Vec::new();
    for (name, s) in &results {
        let mut sorted = s.tick_us.clone();
        sorted.sort_unstable();
        let mean = s.tick_us.iter().map(|&v| u64::from(v)).sum::<u64>() as f64 / s.tick_us.len().max(1) as f64;
        let adv = s.advance_us.iter().map(|&v| u64::from(v)).sum::<u64>() as f64 / s.advance_us.len().max(1) as f64;
        let mut worst: Vec<(u32, usize)> = s.tick_us.iter().copied().zip(0..).collect();
        worst.sort_unstable_by(|a, b| b.cmp(a));
        let worst: Vec<String> = worst.iter().take(10).map(|(us, t)| format!("{t}:{us}")).collect();
        println!(
            "BENCH {name} ticks={} mean_us={mean:.0} advance_mean_us={adv:.0} p50_us={} p95_us={} max_us={} worst10=[{}]{}",
            s.ticks,
            percentile(&sorted, 0.5),
            percentile(&sorted, 0.95),
            sorted.last().copied().unwrap_or(0),
            worst.join(","),
            s.error.as_ref().map(|(t, _)| format!(" sim_error_at={t}")).unwrap_or_default()
        );
        all.extend_from_slice(&s.tick_us);
        all_advance.extend_from_slice(&s.advance_us);
    }
    let mean = all.iter().map(|&v| u64::from(v)).sum::<u64>() as f64 / all.len().max(1) as f64;
    let adv = all_advance.iter().map(|&v| u64::from(v)).sum::<u64>() as f64 / all_advance.len().max(1) as f64;
    all.sort_unstable();
    println!(
        "BENCH TOTAL scenarios={} ticks={} mean_us={mean:.0} advance_mean_us={adv:.0} p50_us={} p95_us={} p99_us={} max_us={} wall_s={:.1}",
        results.len(),
        all.len(),
        percentile(&all, 0.5),
        percentile(&all, 0.95),
        percentile(&all, 0.99),
        all.last().copied().unwrap_or(0),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
