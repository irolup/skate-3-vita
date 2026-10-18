//! Retail-audio stage S0: what skate-audio's render costs on the Vita.
//!
//! Renders the reference's `fma_paths` test scene (crates/skate-audio/tests/fma_paths.rs,
//! `b3c9679`: AEMS voices on every bus with moving filters and azimuths, the eEQChain buses,
//! the FlangeSub returns, the env reverb, a grain truck with its FSS chain; synthetic PCM, no
//! retail data) with 8, 16 and 24 voices, 1,500 blocks each (256 frames at 48 kHz: 32 s of
//! audio), and prints the time per block and a hash of every output bit. The budget is one
//! block per 5.333 ms; the game's audio thread runs on core 2, so the render runs there too.
//! The hash must equal the desktop build's (`cargo run --release -p audio-bench`): both use
//! the no-fma arithmetic.
//!
//! Output lines start with `AUDIO` (scripts/vita-audio-bench.py reads them). Start cancels.

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

use std::sync::Arc;
use std::time::Instant;

use skate_audio::bus::env::{DEFAULT_PRESET, Preset};
use skate_audio::bus::flange::FlangePreset;
use skate_audio::eval::{OpenRequest, VoiceHost};
use skate_audio::formats::SampleHeader;
use skate_audio::grain::player::{GrainParams, GrainSource, Record};
use skate_audio::mixer::Pcm;
use skate_audio::runtime::Runtime;

const BLOCKS: usize = 1500;
const WARMUP: usize = 20;
/// One 256-frame block at 48 kHz.
const BUDGET_US: f64 = 256.0 / 48_000.0 * 1e6;

// The scene and its driver are the reference's fma_paths test (b3c9679), with the voice
// count as a parameter (the test opens 24).
fn reverb01() -> Preset {
    Preset([
        4000.0, 600.0, 1.0, 0.08, 0.0, 1.5, 70.0, 1.0, 0.7, 4000.0, 500.0, 1.0, 1.0, 0.25, 1.0, 1.0, 3.0, 0.1, 0.34, 1.0, 1161.0, 0.5,
        0.2, 0.6, 1.0, 1.0, 270.0, 5000.0, 0.5, 0.45, 500.0, 0.75, 0.3, 0.3, 1.0, 1.0, 90.0, 5000.0, 0.51, 0.5, 500.0, 0.75, 0.28, 0.25,
    ])
}

fn scene(voices: usize) -> (Runtime, Vec<u32>) {
    let mut rt = Runtime::new();
    let frames = 44_100 * 4;
    let wave: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.03).sin() * 0.3).collect();
    let mono = Arc::new(Pcm { rate: 44_100, channels: vec![wave.clone()].into_iter().map(skate_audio::adpcm::Samples::F32).collect() });
    let stereo = Arc::new(Pcm { rate: 44_100, channels: vec![wave.clone(), wave.clone()].into_iter().map(skate_audio::adpcm::Samples::F32).collect() });
    let header = |channels| SampleHeader { codec: 3, channels, rate: 44_100, frames: frames as u32, loop_start: Some(0) };
    rt.mixer.add_bank(0, vec![Some(header(1)), Some(header(2))], vec![Some(mono), Some(stereo)]);
    rt.mixer.buses.env.presets.insert(DEFAULT_PRESET, reverb01());
    rt.mixer.buses.env.request(DEFAULT_PRESET);
    rt.mixer.buses.flange.set_presets(
        FlangePreset([20.0, 0.3, 0.2, 0.1, 1500.0, 0.002, 0.5, 0.9, 0.03]),
        FlangePreset([1.7, 0.3, 0.0, 1.0, 250.0, 0.0005, 0.25, 0.6, 0.11]),
    );
    rt.mixer.buses.flange.frame([32692, 2313, 32692, 2313]);
    for (k, bus) in rt.mixer.buses.eq.buses.iter_mut().enumerate() {
        bus.eq[0].freq = 5000.0 - 400.0 * k as f32;
        bus.eq[0].gain = 1.5;
        bus.eq[0].q = 2.0;
        bus.eq[1].freq = 2000.0;
        bus.eq[1].gain = 0.8;
        bus.eq[1].q = 3.0;
    }
    let routes: Vec<[(u8, i32); 4]> = (0..8).map(|b| [(9u8, b), (10, 4096), (11, 1), (12, 1638)]).collect();
    let ids = (0..voices)
        .filter_map(|k| {
            rt.mixer.open(&OpenRequest { bank: 0, slot: (k % 2) as u16, level: 100, azimuth: [224, 32, 0, 0, 0, 0], stream_offset: u32::MAX, inputs: &routes[k % 8] })
        })
        .collect();
    let source = Arc::new(GrainSource { name: "t".into(), duration: 4.0, pcm: Arc::new(Pcm { rate: 44_100, channels: vec![wave].into_iter().map(skate_audio::adpcm::Samples::F32).collect() }) });
    let a = GrainParams { attack: 0.1, sustain: 0.2, release: 0.1, window: 1.6, drift: 0.05 };
    let b = GrainParams { attack: 0.2, sustain: 0.1, release: 0.2, window: 1.5, drift: 0.05 };
    rt.grains.bind_truck(0, source, [a, b], [Record { gain: 0.5, pitch: 1.0, position: 0.3 }, Record { gain: 0.25, pitch: 1.0, position: 0.2 }]);
    (rt, ids)
}

fn drive(rt: &mut Runtime, ids: &[u32], block: usize) {
    if block.is_multiple_of(6) {
        for (k, &v) in ids.iter().enumerate() {
            rt.mixer.set_azimuth(v, ((block * 37 + k * 911) % 65536) as i32);
            rt.mixer.set(v, 6, 2000 + ((block + k * 13) % 20000) as i32);
            rt.mixer.set(v, 7, 77);
            rt.mixer.set(v, 2, 20000);
            rt.mixer.set(v, 5, 3000);
            rt.mixer.set(v, 0, 4096 + ((block + k) % 400) as i32);
        }
    }
}

fn run() {
    println!("AUDIO START arithmetic no-fma");
    // IEEE mode first (what every thread runs), then flush-to-zero with default NaN
    // (FPSCR FZ + DN, the Vita's own default): denormals trap to support code on the A9.
    for mode in ["ieee", "fz"] {
        set_flush_to_zero(mode == "fz");
        render_all(mode);
    }
    set_flush_to_zero(false);
}

/// FPSCR bits 24 (FZ) and 25 (DN) on the calling thread.
#[cfg(target_os = "vita")]
fn set_flush_to_zero(on: bool) {
    let mut fpscr: u32;
    unsafe { core::arch::asm!("vmrs {}, fpscr", out(reg) fpscr) };
    fpscr = if on { fpscr | 0x0300_0000 } else { fpscr & !0x0300_0000 };
    unsafe { core::arch::asm!("vmsr fpscr, {}", in(reg) fpscr) };
}
#[cfg(not(target_os = "vita"))]
fn set_flush_to_zero(_: bool) {}

fn render_all(mode: &str) {
    for voices in [0, 8, 24] {
        let built = Instant::now();
        let (mut rt, ids) = scene(voices);
        let build_ms = built.elapsed().as_secs_f64() * 1e3;
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut times = Vec::with_capacity(BLOCKS);
        let _ = skate_audio::prof::take();
        for block in 0..WARMUP + BLOCKS {
            drive(&mut rt, &ids, block);
            let t = Instant::now();
            let out = *rt.render_block();
            let us = t.elapsed().as_micros() as u32;
            if block >= WARMUP {
                times.push(us);
            }
            for ch in out.iter() {
                for x in ch.iter() {
                    hash = (hash ^ u64::from(x.to_bits())).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        let mean = times.iter().map(|&t| f64::from(t)).sum::<f64>() / times.len() as f64;
        times.sort_unstable();
        let p = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
        println!(
            "AUDIO mode={mode} voices={voices} opened={} blocks={BLOCKS} mean_us={mean:.0} p50_us={} p95_us={} max_us={} core_pct={:.1} hash={hash:016x} build_ms={build_ms:.0}",
            ids.len(),
            p(0.5),
            p(0.95),
            times[times.len() - 1],
            mean / BUDGET_US * 100.0
        );
        let stages = skate_audio::prof::take();
        if stages.iter().any(|&n| n > 0) {
            let per_block = |n: u64| n as f64 / 1e3 / (WARMUP + BLOCKS) as f64;
            let parts: Vec<String> = skate_audio::prof::NAMES.iter().zip(stages).map(|(n, t)| format!("{n}={:.0}", per_block(t))).collect();
            println!("AUDIO STAGES mode={mode} voices={voices} us_per_block {}", parts.join(" "));
        }
    }
}

fn main() {
    platform_setup();
    let handle = std::thread::Builder::new()
        .name("audio".into())
        .stack_size(1024 * 1024)
        .spawn(|| {
            pin_to_core_2();
            run()
        })
        .expect("spawn audio thread");
    while !handle.is_finished() {
        keep_awake();
        if start_pressed() {
            println!("AUDIO CANCELLED (Start)");
            exit_process();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    match handle.join() {
        Ok(()) => println!("AUDIO DONE"),
        Err(_) => println!("AUDIO ERROR render thread panicked"),
    }
    exit_process();
}

#[cfg(target_os = "vita")]
fn platform_setup() {
    use vitasdk_sys::*;
    unsafe {
        scePowerSetArmClockFrequency(444);
        scePowerSetBusClockFrequency(222);
        scePowerSetGpuClockFrequency(222);
        scePowerSetGpuXbarClockFrequency(166);
        println!("AUDIO clocks arm={} bus={} MHz", scePowerGetArmClockFrequency(), scePowerGetBusClockFrequency());
    }
}
#[cfg(not(target_os = "vita"))]
fn platform_setup() {}

#[cfg(target_os = "vita")]
fn pin_to_core_2() {
    use vitasdk_sys::*;
    let ret = unsafe { sceKernelChangeThreadCpuAffinityMask(sceKernelGetThreadId(), SCE_KERNEL_CPU_MASK_USER_2 as i32) };
    let fpscr: u32;
    unsafe { core::arch::asm!("vmrs {}, fpscr", out(reg) fpscr) };
    println!("AUDIO render thread on core 2 ({ret:#x}), fpscr={fpscr:#010x}");
}
#[cfg(not(target_os = "vita"))]
fn pin_to_core_2() {}

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

#[cfg(target_os = "vita")]
fn exit_process() -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::thread::sleep(std::time::Duration::from_millis(300));
    unsafe { vitasdk_sys::sceKernelExitProcess(0) };
    std::process::exit(0)
}
#[cfg(not(target_os = "vita"))]
fn exit_process() -> ! {
    std::process::exit(0)
}
