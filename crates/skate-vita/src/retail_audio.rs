//! The retail board audio: the reference's audio host (`game_audio`) fed from
//! the simulation and rendered on the audio thread from the baked `sound.pak`.
//!
//! The simulation thread publishes into [`MAILBOX`] after its ticks; a host thread (core 0, low
//! priority) runs the host pass (`Native::frame`) every 1/60 s; the audio thread (core 2) only
//! renders.
//! Without `sound.pak` the procedural synth of `crate::audio` plays instead.
use std::sync::Mutex;
use std::time::Instant;

use game_audio::library::Library;
use game_audio::native::{FrameInput, Listener, Native, Renderer};

pub const PACK: &str = "ux0:data/SKATEVITA/sound.pak";
/// The virtual folder the pack's files are mounted under.
pub const ROOT: &str = "vfs:/sound";

/// What the simulation hands the host: the skater's audio state, the physics steps since the
/// audio thread last took them, the camera and the cut counter.
pub struct Mailbox {
    pub riding: Option<skate_sim::AudioRiding>,
    pub steps: u32,
    pub listener: Option<Listener>,
    pub cuts: u64,
    /// Paused or in a menu: the host fades the board sounds out.
    pub silenced: bool,
    /// Front-end records to play (the session marker's cellphone sounds), by key.
    pub frontend: Vec<u64>,
    /// The loaded map (its retail name) and a number that changes with every load: the zone beds.
    pub map: Option<(String, u64)>,
    /// The teleport effect amount (the Go To Marker hold, 0..=1), when there is one.
    pub teleport: Option<f32>,
    /// Ticks left of the relocation's full amount (retail sends 1.0 for the relocation tick and two more).
    teleport_hold: u8,
}

pub static MAILBOX: Mutex<Mailbox> =
    Mutex::new(Mailbox { riding: None, steps: 0, listener: None, cuts: 0, silenced: true, frontend: Vec::new(), map: None, teleport: None, teleport_hold: 0 });

/// Set once the host has the front-end sounds: the synth's marker cues stay silent then.
static FRONTEND_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn frontend_ready() -> bool {
    FRONTEND_READY.load(std::sync::atomic::Ordering::Relaxed)
}

/// A world is loaded: the zone beds follow it (`name` is the retail map name, `generation` counts loads).
pub fn set_map(name: &str, generation: u64) {
    MAILBOX.lock().unwrap().map = Some((name.to_owned(), generation));
}

/// A session-marker audio id from the simulation (`Report::marker_audio`): the retail sound for it.
pub fn marker_sound(id: u64) {
    let Some(key) = game_audio::frontend_marker::marker_record(id) else { return };
    let mut m = MAILBOX.lock().unwrap();
    // Bounded in case the host is not draining it.
    if m.frontend.len() < 16 {
        m.frontend.push(key);
    }
    if id == skate_sim::AUDIO_RETURN {
        m.teleport_hold = 3;
    }
}

/// The render handle, handed from the loader thread to the audio thread.
static READY: Mutex<Option<Renderer>> = Mutex::new(None);

/// The most voices that may sound at once on the Vita (the reference allows 96; retail retires
/// voices by CPU). 24: a profile peaked at 58 with the render over budget, and
/// 32 still left the render at 4–4.8 ms of each 5.3 ms block. 16 (owner, 2026-10-27): at 20 the render took 5.2 ms
/// of each 5.33 ms block on Industrial's roads (about 1.2 ms plus 0.2 ms a voice) and starved the tile streaming
/// on the same core.
pub const MAX_VOICES: usize = 16;

const FRAME: f32 = 1.0 / 60.0;

/// Called by the simulation thread after its ticks.
pub fn publish(session: &mut skate_sim::live::Session) -> skate_sim::AudioRiding {
    let (riding, steps) = session.take_audio();
    let listener = session.presentation().map(|(_, now)| {
        let t = now.camera;
        Listener { position: t.translation, forward: (t.rotation * glam::Vec3::NEG_Z).normalize_or_zero() }
    });
    let cuts = session.presentation_cuts();
    let progress = session.marker_view().progress;
    let mut m = MAILBOX.lock().unwrap();
    // The hold's progress while it runs, then 1.0 for the relocation and two more ticks.
    m.teleport = if m.teleport_hold > 0 {
        m.teleport_hold -= 1;
        Some(1.0)
    } else {
        (progress > 0.0).then_some(progress.min(1.0))
    };
    m.riding = Some(riding);
    m.steps += steps;
    m.listener = listener;
    m.cuts = cuts;
    m.silenced = false;
    riding
}

/// Replay playback (the main thread feeds the host while the simulation is held).
static REPLAYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_replaying(on: bool) {
    REPLAYING.store(on, std::sync::atomic::Ordering::Relaxed);
    if !on {
        silence();
    }
}

/// One replay frame's audio: the recorded state, the ticks played since the last call and the
/// camera; `cuts` changes on every jump so the host resets.
pub fn publish_replay(riding: skate_sim::AudioRiding, steps: u32, camera: skate_sim::live::Transform, cuts: u64, silenced: bool) {
    let mut m = MAILBOX.lock().unwrap();
    m.riding = Some(riding);
    m.steps += steps;
    m.listener = Some(Listener { position: camera.translation, forward: (camera.rotation * glam::Vec3::NEG_Z).normalize_or_zero() });
    m.cuts = cuts;
    m.silenced = silenced;
}

/// Called by the simulation thread when it stops stepping (pause, menus, reload).
pub fn silence() {
    if REPLAYING.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    MAILBOX.lock().unwrap().silenced = true;
}

/// Banks in sound.pak the board host never loads (the front end and the world's water): not
/// read on the Vita until the world and front-end audio use them.
const UNUSED_BANKS: [&str; 6] =
    ["ocean_wave_small", "fountains_waterlaps_left", "water_fountain", "water_lapping", "water_lapping_pond", "water_misc"];
/// Only read while loading: freed once the host has started.
const MANIFEST: &str = "private/audio/audio_manifest.json";
/// The zone beds (vita-bake ambience), streamed from the card.
pub const AMBIENCE_PACK: &str = "ux0:data/SKATEVITA/ambience.pak";
/// The emitters' banks (vita-bake audio), read by proximity from the card.
pub const WORLD_PACK: &str = "ux0:data/SKATEVITA/world.pak";
/// Voices the world sounds (emitters, beds) leave free for the board's: at most `MAX_VOICES - this` play for them.
const WORLD_RESERVE: usize = 8;

fn unused(name: &str) -> bool {
    UNUSED_BANKS.iter().any(|b| name.contains(&format!("/banks/{b}/")) || name.ends_with(&format!("/{b}.abk")))
        // The grains' speed bands (`grains/<surface>/<n>.ima`): the host plays the whole
        // recordings (`grains/<surface>.ima`); library.rs ignores the bands. 6.7 MB.
        || (name.contains("/grains/") && name.rsplit('/').nth(1).is_some_and(|dir| dir != "grains"))
}

/// Mounts `sound.pak` (one buffer for what stays resident), loads the library and the host,
/// hands the renderer to the audio thread and runs the host pass on this thread (core 0, low
/// priority) every 1/60 s. Returns only on a load error.
pub fn load_and_run() -> Result<(), String> {
    let started = Instant::now();
    let (files, resident) = crate::packs::mount_selected(PACK, ROOT, |n| !unused(n), |n| n == MANIFEST)?;
    let mounted = started.elapsed().as_secs_f32();
    // The beds stay on the card; without the pack the world has no ambience.
    // The stream thread reads 36 KiB a second per bed channel: tiny, but a missed deadline is
    // silence in the bed, so it runs above the game's threads (which starved it at 170: 5,700
    // underruns in one busy stretch). The bank worker's reads are 1 MB bursts and stay low.
    skate_audio::stream::set_thread_init(|| {
        let _ = vita_platform::vita::allow_current_thread_on(&[0, 2]);
        vita_platform::vita::set_current_thread_priority(100);
    });
    game_audio::set_bank_worker_init(|| {
        let _ = vita_platform::vita::allow_current_thread_on(&[0]);
        vita_platform::vita::set_current_thread_priority(170);
    });
    match crate::packs::mount_on_card(AMBIENCE_PACK, ROOT) {
        Ok(n) => println!("ambience.pak: {n} beds on the card"),
        Err(e) => println!("{AMBIENCE_PACK}: {e}; the world has no ambience"),
    }
    match crate::packs::mount_on_card(WORLD_PACK, ROOT) {
        Ok(n) => println!("world.pak: {n} files on the card"),
        Err(e) => println!("{WORLD_PACK}: {e}; the world has no emitters"),
    }
    let library = Library::load(std::path::Path::new(ROOT))?;
    let mut native = Native::start(&library)?;
    native.set_max_voices(MAX_VOICES);
    native.set_world_reserve(WORLD_RESERVE);
    FRONTEND_READY.store(native.has_frontend(), std::sync::atomic::Ordering::Relaxed);
    skate_data::vfs::unmount(&std::path::Path::new(ROOT).join(MANIFEST));
    println!(
        "sound.pak: {files} files ({:.1} MiB resident) mounted in {mounted:.1} s, host ready in {:.1} s",
        resident as f64 / 1_048_576.0,
        started.elapsed().as_secs_f32()
    );
    *READY.lock().unwrap() = Some(native.renderer());
    let period = std::time::Duration::from_secs_f32(FRAME);
    let mut next = Instant::now();
    let mut last = Instant::now();
    let (mut frames, mut total_us, mut max_us, mut window) = (0u32, 0u64, 0u32, Instant::now());
    let mut emitters_max = 0usize;
    loop {
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > period * 4 {
            next = now;
        }
        let real_dt = last.elapsed().as_secs_f32();
        last = Instant::now();
        let (riding, steps, listener, cuts, silenced, frontend, teleport, map) = {
            let mut m = MAILBOX.lock().unwrap();
            (m.riding, std::mem::take(&mut m.steps), m.listener, m.cuts, m.silenced, std::mem::take(&mut m.frontend), m.teleport, m.map.clone())
        };
        let Some(riding) = riding else { continue };
        let t = Instant::now();
        native.frame(&library, FrameInput {
            riding: &riding,
            steps,
            listener,
            cuts: (cuts, 0),
            overstep: 0.0,
            game_dt: steps as f32 * FRAME,
            real_dt,
            silenced,
            frontend: &frontend,
            teleport,
            map: map.as_ref().map(|(n, g)| (n.as_str(), *g)),
        });
        let us = t.elapsed().as_micros() as u32;
        frames += 1;
        total_us += u64::from(us);
        max_us = max_us.max(us);
        emitters_max = emitters_max.max(native.emitters_playing());
        if window.elapsed().as_secs() >= 10 {
            println!(
                "AUDIO host {:.2} ms/frame (max {:.2}) over {frames} frames; emitters max {emitters_max}, stream underruns {}",
                total_us as f64 / 1000.0 / f64::from(frames.max(1)),
                f64::from(max_us) / 1000.0,
                skate_audio::stream::underruns_total()
            );
            emitters_max = 0;
            (frames, total_us, max_us, window) = (0, 0, 0, Instant::now());
        }
    }
}

/// Per-block render cost, logged every 10 s of output.
#[derive(Default)]
struct Cost {
    blocks: u32,
    render_us: u64,
    render_max: u32,
    voices_max: usize,
}

impl Cost {
    fn log(&mut self) {
        if self.blocks < 1875 {
            return;
        }
        println!(
            "AUDIO render {:.2} ms/block (max {:.2}), voices max {} over {} blocks",
            self.render_us as f64 / 1000.0 / f64::from(self.blocks),
            f64::from(self.render_max) / 1000.0,
            self.voices_max,
            self.blocks,
        );
        #[cfg(feature = "audio-profile")]
        {
            let stages = skate_audio::prof::take();
            let parts: Vec<String> = skate_audio::prof::NAMES
                .iter()
                .zip(stages)
                .map(|(n, t)| format!("{n}={:.0}", t as f64 / 1000.0 / f64::from(self.blocks)))
                .collect();
            println!("AUDIO stages us/block: {}", parts.join(" "));
        }
        *self = Cost::default();
    }
}

/// The audio thread's side once the host is ready: renders only.
pub struct Player {
    renderer: Renderer,
    mix: Vec<f32>,
    cost: Cost,
}

impl Player {
    /// The renderer, once the loader thread has finished.
    pub fn take_ready() -> Option<Self> {
        let renderer = READY.lock().unwrap().take()?;
        Some(Self { renderer, mix: Vec::new(), cost: Cost::default() })
    }

    /// One output block of interleaved stereo (`out_frames` frames at 48 kHz) as f32.
    pub fn fill(&mut self, out_frames: usize) -> &[f32] {
        self.mix.resize(out_frames * 2, 0.0);
        let started = Instant::now();
        self.renderer.render(&mut self.mix);
        let us = started.elapsed().as_micros() as u32;
        self.cost.blocks += 1;
        self.cost.render_us += u64::from(us);
        self.cost.render_max = self.cost.render_max.max(us);
        self.cost.voices_max = self.cost.voices_max.max(self.renderer.voices());
        self.cost.log();
        &self.mix
    }
}
