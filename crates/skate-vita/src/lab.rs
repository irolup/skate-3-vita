//! The live simulation thread and the stick calibration screen
//! (the input lab): raw and converted sticks, each stick's converted magnitude
//! per direction, touch zones, the Xbox buttons the mapping sends, and the tricks the
//! score module recognizes, with counters for the 20/20 acceptance test.
//!
//! The simulation runs on core 1 in its own thread with a fixed-step
//! accumulator (stock `clock.rs` period, at most 1 + 2 ticks per wake-up, then time
//! is dropped and counted). It polls the controls once per tick and publishes each
//! tick's presentation snapshots for the renderer. The main thread (`app.rs`)
//! draws the 3D view or this screen at 60 fps.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use skate_core::input::xbox::{XboxState, convert};
use skate_sim::live::{Session, Snapshot, Trick};
use vita_platform::controls::{self, Settings, xbox};
use vita_platform::vita::{self, Input, Sample};

use vita_render::ui::*;

/// Input recordings (sim-oracle `.input` format) of sessions that failed.
const RECORDINGS: &str = "ux0:data/SKATEVITA/recordings";
const SECTORS: usize = 32;
/// A gap this long between wake-ups means the app was suspended: restart the clock
/// instead of catching up.
const SUSPEND_GAP: Duration = Duration::from_millis(250);

/// The six tricks of the input lab's acceptance test.
const TARGETS: [&str; 6] = ["Ollie", "Kickflip", "Heelflip", "360 Flip", "Hardflip", "Nollie Flip"];

fn target(trick: &Trick) -> Option<usize> {
    let label = trick.label.as_str();
    match label {
        "ID_TRICK_FLIP_OLLIE" if !trick.nollie => Some(0),
        "ID_TRICK_FLIP_KICKFLIP" if !trick.nollie => Some(1),
        "ID_TRICK_FLIP_HEELFLIP" if !trick.nollie => Some(2),
        "ID_TRICK_FLIP_360_FLIP" if !trick.nollie => Some(3),
        "ID_TRICK_FLIP_HARDFLIP" if !trick.nollie => Some(4),
        "ID_TRICK_FLIP_NOLLIE_KICKFLIP" => Some(5),
        "ID_TRICK_FLIP_KICKFLIP" if trick.nollie => Some(5),
        _ => None,
    }
}

fn display(trick: &Trick) -> String {
    let name = trick.label.strip_prefix("ID_TRICK_").unwrap_or(&trick.label);
    let stance = [(trick.switch, "S"), (trick.fakie, "F"), (trick.nollie, "N")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, s)| *s)
        .collect::<String>();
    if stance.is_empty() {
        name.to_owned()
    } else {
        format!("{name} [{stance}]")
    }
}

#[derive(Clone, Copy)]
struct StickStats {
    /// Largest converted magnitude seen per direction sector.
    sector_max: [f32; SECTORS],
    raw_min: [u8; 2],
    raw_max: [u8; 2],
}
impl Default for StickStats {
    fn default() -> Self {
        Self {
            sector_max: [0.0; SECTORS],
            raw_min: [255; 2],
            raw_max: [0; 2],
        }
    }
}
impl StickStats {
    fn add(&mut self, raw: [u8; 2], converted: [f32; 2]) {
        for i in 0..2 {
            self.raw_min[i] = self.raw_min[i].min(raw[i]);
            self.raw_max[i] = self.raw_max[i].max(raw[i]);
        }
        let magnitude = (converted[0] * converted[0] + converted[1] * converted[1]).sqrt();
        if magnitude > 0.0 {
            let angle = converted[1].atan2(converted[0]).rem_euclid(core::f32::consts::TAU);
            let sector = ((angle / core::f32::consts::TAU * SECTORS as f32) as usize).min(SECTORS - 1);
            self.sector_max[sector] = self.sector_max[sector].max(magnitude);
        }
    }
    fn weakest(&self) -> f32 {
        self.sector_max.iter().copied().fold(f32::INFINITY, f32::min)
    }
}

/// One tick's presentation pair, due at `due` (when the tick ran), shown for the
/// following `period` (interpolation).
pub(crate) struct Published {
    pub due: Instant,
    pub period: Duration,
    pub previous: Snapshot,
    pub current: Snapshot,
    /// The traffic cars of the same tick (their previous and current poses).
    pub cars: Vec<skate_sim::CarPose>,
    /// The NPC skaters of the same tick (their previous and current poses, opacity).
    pub npcs: Vec<skate_sim::NpcPose>,
    /// The peds in view of the same tick.
    pub peds: Vec<skate_sim::PedPose>,
    /// The released hand props of the same tick.
    pub hand_props: Vec<skate_sim::HandPropPose>,
}
/// Published ticks kept for the renderer.
const RING: usize = 4;

#[derive(Default)]
pub(crate) struct Shared {
    // Controls (written by the sim thread every tick).
    pub sample: Option<Sample>,
    buttons: u16,
    triggers: [u8; 2],
    converted: [[f32; 2]; 2],
    sticks: [StickStats; 2],
    front_area: Option<controls::Area>,
    rear_area: Option<controls::Area>,
    pub settings: Settings,
    // Simulation.
    pub loading: bool,
    pub error: Option<String>,
    pub state: String,
    animation: String,
    pub speed: f32,
    pub ticks: u64,
    pub tick_us: u32,
    pub tick_us_max: u32,
    pub dropped: u64,
    pub pauses: u64,
    fpscr: u32,
    counts: [u32; TARGETS.len()],
    other: u32,
    log: std::collections::VecDeque<String>,
    /// Last saved recording of a failed session.
    saved: Option<String>,
    // Rendering.
    pub frames: std::collections::VecDeque<Published>,
    pub bone_names: Option<Vec<String>>,
    pub last_trick: Option<String>,
    // Worlds: the selected entry of `packs::WORLDS`, the loaded one, and a park's
    // render data handed to the main thread (None = the Course).
    pub world: usize,
    pub world_loaded: usize,
    pub park_render: Option<(usize, Option<vita_pack::map::Render>)>,
    /// The loaded world's movable props render (`props.pak`), for the main thread; `Some(None)`: none.
    pub props_render: Option<Option<vita_pack::map::PropRender>>,
    /// The props streamed in, at the last tick's poses.
    pub props: Vec<skate_sim::live::PropPose>,
    /// The loaded world's car models for the main thread (`Session::car_models` order; an
    /// empty list: no traffic).
    pub cars_render: Option<Vec<Option<vita_pack::skater::Skater>>>,
    /// The loaded world's ped models (`Session::ped_models` order) and the ped rig (bone names,
    /// parents, animated flags) for the main thread; an empty list: no peds.
    pub peds_render: Option<(Vec<Option<vita_pack::skater::Skater>>, Option<(Vec<String>, Vec<i32>, Vec<bool>)>)>,
    /// The hand prop models (by `Session::hand_prop_models` index) for the renderer.
    pub hand_props_render: Option<Vec<Option<vita_pack::skater::Skater>>>,
    /// The living world's settings (traffic, density, NPC draw distance), applied at the
    /// next load and live when `living_changed` is set.
    pub living: skate_sim::LivingWorldSettings,
    pub living_changed: bool,
    /// Population counts (skaters, peds, cars) for the log.
    pub living_counts: Option<[usize; 3]>,
    // Game.
    /// The difficulty to load with (and to apply live when it changes).
    pub difficulty: crate::settings::Difficulty,
    /// The solve's constraint iterations (the Physics setting: 25 Fast, 50 Retail).
    pub solver_iterations: u32,
    /// No stepping while set (menus); the clock restarts on resume.
    pub paused: bool,
    /// Request: teleport the skater to the world's spawn.
    pub respawn: bool,
    pub score: skate_sim::live::Score,
    pub state_id: u32,
    /// The session marker overlay's state.
    pub marker: skate_sim::MarkerView,
    /// The manual meter's input.
    pub manual: skate_sim::ManualView,
    /// The original HUD movie's input, one entry per simulation tick; the main
    /// thread plays them in order. `hud_generation` changes when the session (re)loads.
    pub hud_feeds: std::collections::VecDeque<skate_sim::live::HudFeed>,
    pub hud_latest: skate_sim::live::HudFeed,
    pub hud_generation: u64,
    /// Decoded stock animation bytes (bounded by `live::ANIMATION_BUDGET`).
    pub animation_bytes: usize,
    /// Set while a failed session reloads.
    pub recovering: bool,
    // Requests from the main thread.
    pub reset: bool,
    pub reload: bool,
    /// Request to the main thread: exit the app (autoplay finished).
    pub quit: bool,
}

pub(crate) fn sim_thread(shared: &Mutex<Shared>) {
    crate::alloc::tag_thread("sim");
    let pinned = vita::pin_current_thread(1);
    let input = Input::new();
    {
        let mut s = shared.lock().unwrap();
        s.fpscr = vita::fpscr();
        s.front_area = Some(input.front_area);
        s.rear_area = Some(input.rear_area);
    }
    println!("sim thread: core 1 ({pinned:#x}), fpscr {:#010x}, panels {:?} / {:?}", vita::fpscr(), input.front_area, input.rear_area);
    let assets = std::path::Path::new(crate::packs::mount());
    skate_sim::live::enable_game_switches();
    #[cfg(feature = "replay")]
    crate::autoplay::run(shared, assets);
    loop {
        {
            let mut s = shared.lock().unwrap();
            s.loading = true;
            s.reload = false;
        }
        let started = Instant::now();
        let (world, difficulty) = {
            let s = shared.lock().unwrap();
            (s.world, s.difficulty.key())
        };
        let name = crate::packs::WORLDS[world];
        let session = if world == 0 {
            crate::packs::clear_district();
            shared.lock().unwrap().park_render = Some((0, None));
            Session::load(assets, difficulty)
        } else {
            match crate::packs::load_park(name) {
                Ok((sim, render)) => {
                    shared.lock().unwrap().park_render = Some((world, Some(render)));
                    Session::load_map(assets, difficulty, Some(sim))
                }
                Err(e) => Err(e),
            }
        };
        // The world's movable props render, when props.pak has it.
        let props_render = (world != 0)
            .then(|| skate_data::vfs::read(&std::path::Path::new(assets).join(format!("private/native-props/{name}.render"))).ok())
            .flatten()
            .and_then(|bytes| match vita_pack::map::PropRender::from_pack(&bytes) {
                Ok(r) => Some(r),
                Err(e) => {
                    println!("props render {name}: {e}");
                    None
                }
            });
        shared.lock().unwrap().props_render = Some(props_render);
        let mut session = match session {
            Ok(mut session) => {
                crate::alloc::stage(&format!("{name}: session loaded (before the living world)"));
                if let Some((props, awake)) = session.prop_counts() {
                    println!("props: {props} placed, {awake} streamed in");
                }
                // The living world (districts) and its car models for the renderer.
                let settings = shared.lock().unwrap().living.clone();
                let cars = if session.start_living_world(settings) {
                    let models: Vec<Option<vita_pack::skater::Skater>> = session
                        .car_models()
                        .iter()
                        .map(|glb| {
                            let path = std::path::Path::new(assets).join(glb.strip_suffix(".glb").unwrap_or(glb)).with_extension("car");
                            match skate_data::vfs::read(&path).map_err(|e| e.to_string()).and_then(|b| vita_pack::skater::Skater::from_pack(&b)) {
                                Ok(m) => Some(m),
                                Err(e) => {
                                    println!("car model {}: {e}", path.display());
                                    None
                                }
                            }
                        })
                        .collect();
                    println!("living world: {} car models", models.iter().flatten().count());
                    models
                } else {
                    Vec::new()
                };
                // The ped models.
                let ped_models = session.ped_models();
                let peds: Vec<Option<vita_pack::skater::Skater>> = ped_models
                    .iter()
                    .map(|recipe| {
                        let path = std::path::Path::new(assets).join(format!("private/living_world/models/{recipe}.ped"));
                        skate_data::vfs::read(&path).ok().and_then(|b| vita_pack::skater::Skater::from_pack(&b).map_err(|e| println!("ped model {recipe}: {e}")).ok())
                    })
                    .collect();
                if !ped_models.is_empty() {
                    println!("living world: {} ped models of {}", peds.iter().flatten().count(), ped_models.len());
                }
                // The hand prop models.
                let hand_models = session.hand_prop_models();
                let hands: Vec<Option<vita_pack::skater::Skater>> = hand_models
                    .iter()
                    .map(|path| skate_data::vfs::read(&std::path::Path::new(assets).join(path)).ok().and_then(|b| vita_pack::skater::Skater::from_pack(&b).map_err(|e| println!("hand prop model {path}: {e}")).ok()))
                    .collect();
                {
                    // The guard ends here: the lines below lock the shared state again.
                    let mut s = shared.lock().unwrap();
                    s.cars_render = Some(cars);
                    s.peds_render = Some((peds, session.ped_rig()));
                    s.hand_props_render = Some(hands);
                }
                println!("session loaded in {:.1} s: {name}", started.elapsed().as_secs_f32());
                crate::alloc::stage(&format!("{name}: living world and its models loaded"));
                static LOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                crate::retail_audio::set_map(name, LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1);
                shared.lock().unwrap().world_loaded = world;
                session
            }
            Err(error) => {
                println!("session load failed: {error}");
                let mut s = shared.lock().unwrap();
                s.loading = false;
                s.error = Some(format!("load: {error}"));
                drop(s);
                wait_for_reload(shared, &input);
                continue;
            }
        };
        {
            let mut s = shared.lock().unwrap();
            s.loading = false;
            s.recovering = false;
            s.error = None;
            s.frames.clear();
            s.bone_names = Some(session.bone_names().to_vec());
            s.respawn = false;
            s.hud_feeds.clear();
            s.hud_latest = session.hud_feed();
            s.hud_generation += 1;
        }
        // Manual session markers: LB shows the prompt, LB + D-pad down places, LB +
        // D-pad up held returns (not in the scripted soak, whose pad is a recording).
        #[cfg(not(feature = "soak"))]
        if let Err(e) = session.set_session_markers(true) {
            println!("session markers unavailable: {e}");
        }
        run_session(shared, &input, &mut session, difficulty, name);
    }
}

/// While nothing runs, keep the controls display alive until a reload is requested.
fn wait_for_reload(shared: &Mutex<Shared>, input: &Input) {
    loop {
        let sample = input.poll();
        let mut s = shared.lock().unwrap();
        s.sample = Some(sample);
        if s.reload {
            return;
        }
        drop(s);
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// The bound of the decoded animation keys on the Vita (see `run_session`).
const ANIMATION_BUDGET: usize = 16 << 20;

fn run_session(shared: &Mutex<Shared>, input: &Input, session: &mut Session, mut difficulty: &'static str, map: &str) {
    let mut next = Instant::now();
    let mut cues = crate::audio::Cues::default();
    let mut events = Vec::new();
    let mut last_wake = next;
    let mut second = Instant::now();
    let mut max_this_second = 0u32;
    // Every tick's pad state, so a failed session can be replayed on the desktop
    // (the simulation is deterministic): buttons, triggers, left, right.
    let mut recording: Vec<[i32; 7]> = Vec::with_capacity(60 * 60 * 30);
    // This tick's cars, copied into the published ring entry.
    let mut cars: Vec<skate_sim::CarPose> = Vec::new();
    // This tick's NPC skaters.
    let mut npcs: Vec<skate_sim::NpcPose> = Vec::new();
    // This tick's peds in view, and the ped model list `PedPose::model` indexes.
    let mut peds: Vec<skate_sim::PedPose> = Vec::new();
    let ped_models = session.ped_models();
    // This tick's released hand props.
    let mut hand_props: Vec<skate_sim::HandPropPose> = Vec::new();
    // The Physics setting, applied before the first tick and whenever it changes.
    let mut iterations = shared.lock().unwrap().solver_iterations;
    session.set_solver_iterations(iterations);
    // The decoded animation keys: 16 MiB instead of 24 since the retail audio (the districts
    // are at the heap's limit; a trimmed clip decodes again to the same words).
    session.set_animation_budget(ANIMATION_BUDGET);
    crate::session_replay::clear();
    println!("physics: {iterations} solver iterations");
    let mut paused_since: Option<Instant> = None;
    #[cfg(feature = "state-log")]
    let mut last_state = String::new();
    loop {
        // Menus: no stepping, silence. A world that has not ticked yet takes one tick
        // with a neutral pad so it shows behind the menu.
        let mut neutral = false;
        {
            let mut s = shared.lock().unwrap();
            if s.reload {
                crate::retail_audio::silence();
                return;
            }
            if s.paused {
                if s.frames.is_empty() && session.ticks() < 8 {
                    neutral = true;
                } else {
                    paused_since.get_or_insert_with(Instant::now);
                    drop(s);
                    crate::retail_audio::silence();
                    let mut feed = crate::audio::FEED.lock().unwrap();
                    (feed.params.rolling, feed.params.grinding, feed.params.sliding) = (0.0, 0.0, 0.0);
                    drop(feed);
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
            }
            // Resume: the clock restarts (no catch-up) and the published ticks move
            // forward by the pause, so the interpolation continues without a hold.
            if let Some(since) = paused_since.take() {
                let gap = since.elapsed();
                for e in s.frames.iter_mut() {
                    e.due += gap;
                }
                next = Instant::now();
                last_wake = next;
                session.suspend_session_markers();
            }
            if s.solver_iterations != iterations {
                iterations = s.solver_iterations;
                session.set_solver_iterations(iterations);
                println!("physics: {iterations} solver iterations (live)");
            }
            if std::mem::take(&mut s.living_changed) {
                session.set_living_settings(s.living.clone());
                println!("living world: {:?} (live)", s.living);
            }
            s.living_counts = session.living_counts();
            if let (Some([skaters, peds, cars]), 0) = (s.living_counts, session.ticks() % 600) {
                println!("LIVING skaters={skaters} peds={peds} cars={cars} tick={} {}", session.ticks(), session.ped_brain_summary());
            }
            if s.difficulty.key() != difficulty {
                difficulty = s.difficulty.key();
                if let Err(e) = session.set_difficulty(difficulty) {
                    println!("difficulty: {e}");
                }
                println!("difficulty: {difficulty} (live)");
            }
            if std::mem::take(&mut s.respawn) {
                if let Err(e) = session.respawn() {
                    println!("respawn: {e}");
                }
            }
        }
        let now = Instant::now();
        if now - last_wake > SUSPEND_GAP {
            println!("SUSPEND sim thread: {} ms gap, clock restarted", (now - last_wake).as_millis());
            next = now;
            shared.lock().unwrap().pauses += 1;
        }
        last_wake = now;
        let period = session.period();
        let mut due = if neutral {
            next = now;
            1
        } else if now >= next {
            (now - next).as_nanos() / period.as_nanos() + 1
        } else {
            0
        } as u64;
        if due > 3 {
            let dropped = due - 3;
            next += period * dropped as u32;
            shared.lock().unwrap().dropped += dropped;
            due = 3;
        }
        for _ in 0..due {
            #[cfg_attr(feature = "soak", allow(unused_variables))]
            let settings = shared.lock().unwrap().settings;
            let sample = input.poll();
            let state = if neutral {
                XboxState { buttons: 0, triggers: [0, 0], left: [0, 0], right: [0, 0] }
            } else {
                #[cfg(feature = "soak")]
                let scripted = crate::soak::state(shared, session);
                #[cfg(not(feature = "soak"))]
                let scripted = input.xbox(&sample, &settings);
                scripted
            };
            let values = convert(&state, 0);
            let converted = [
                [values[16] - values[17], values[18] - values[19]],
                [values[20] - values[21], values[22] - values[23]],
            ];
            let (buttons, triggers) = (state.buttons, state.triggers);
            // The first 30 minutes (preallocated; nothing grows after that).
            if recording.len() < recording.capacity() {
                recording.push([
                    i32::from(state.buttons),
                    i32::from(state.triggers[0]),
                    i32::from(state.triggers[1]),
                    i32::from(state.left[0]),
                    i32::from(state.left[1]),
                    i32::from(state.right[0]),
                    i32::from(state.right[1]),
                ]);
            }
            let tick_start = Instant::now();
            let report = session.step(state);
            #[cfg(feature = "stage-profile")]
            if session.ticks() % 600 == 0 {
                let mut stages = skate_sim::profile_take();
                stages.retain(|s| s.0.starts_with("z_"));
                stages.sort_by(|a, b| b.1.cmp(&a.1));
                // Counters (`_n`) as a mean per tick, timers in ms per tick.
                let line: Vec<String> = stages.iter().map(|(n, v, _)| if n.ends_with("_n") { format!("{n}={:.1}", *v as f64 / 600.0) } else { format!("{n}={:.2}", *v as f64 / 600.0 / 1e6) }).collect();
                println!("STAGES ms/tick over 600 ticks: {} (state {})", line.join(" "), session.state_name());
                let spans: Vec<String> = crate::span_profile::PROFILER.take().iter().take(16).map(|(n, t, _)| format!("{n}={:.2}", t.as_secs_f64() * 1e3 / 600.0)).collect();
                println!("SPANS ms/tick: {}", spans.join(" "));
            }
            if !neutral {
                crate::session_replay::record(session);
            }
            #[cfg(feature = "state-log")]
            {
                let state = session.state_name();
                if state != last_state {
                    println!("STATE tick {} {last_state} -> {state} anim {}", session.ticks(), session.animation_name().unwrap_or("-"));
                    last_state = state;
                }
            }
            let tick_us = tick_start.elapsed().as_micros() as u32;
            max_this_second = max_this_second.max(tick_us);
            let due_at = next;
            next += session.period();

            // Sound cues from this tick.
            let (deck, speed) = session.board();
            let id = session.state_id();
            for id in &report.marker_audio {
                // The retail audio plays the front-end records; the synth keeps its cues until it has them.
                crate::retail_audio::marker_sound(*id);
                events.push(match *id {
                    skate_sim::AUDIO_SET => crate::audio::Event::MarkerSet,
                    skate_sim::AUDIO_RETURN => crate::audio::Event::MarkerReturn,
                    skate_sim::AUDIO_DENIED => crate::audio::Event::MarkerDenied,
                    _ => continue,
                });
            }
            let (rolling, grinding, sliding) = cues.tick(id, deck[1], speed, session.period().as_secs_f32(), &mut events);
            {
                let mut feed = crate::audio::FEED.lock().unwrap();
                feed.params.rolling = rolling;
                feed.params.grinding = grinding;
                feed.params.sliding = sliding;
                // Bounded in case the audio thread is not draining it.
                if feed.events.len() < 64 {
                    feed.events.append(&mut events);
                }
                events.clear();
            }
            let mut s = shared.lock().unwrap();
            if let Some((previous, current)) = session.presentation() {
                session.car_poses(&mut cars);
                session.npc_poses(&mut npcs);
                session.ped_poses(&ped_models, &mut peds);
                session.hand_prop_poses(&mut hand_props);
                publish(&mut s.frames, due_at, session.period(), previous, current, &cars, &npcs, &peds, &hand_props);
                session.prop_poses(&mut s.props);
            }
            push_hud(&mut s, session.hud_feed());
            s.marker = session.marker_view();
            s.manual = session.manual_view();
            s.state_id = id;
            s.speed = speed;
            s.score = session.score();
            s.sample = Some(sample);
            s.buttons = buttons;
            s.triggers = triggers;
            s.converted = converted;
            s.sticks[0].add([sample.pad.lx, sample.pad.ly], converted[0]);
            s.sticks[1].add([sample.pad.rx, sample.pad.ry], converted[1]);
            s.ticks = session.ticks();
            s.tick_us = tick_us;
            if let Some(trick) = report.new_trick {
                match target(&trick) {
                    Some(i) => s.counts[i] += 1,
                    None => s.other += 1,
                }
                let line = format!("{:>6} {}", session.ticks(), display(&trick));
                println!("trick {line}");
                s.last_trick = Some(display(&trick));
                s.log.push_front(line);
                s.log.truncate(14);
            }
            if let Some(error) = report.error {
                println!("simulation error at tick {}: {error}", session.ticks());
                s.error = Some(error);
            }
            if s.reset {
                s.reset = false;
                s.counts = Default::default();
                s.other = 0;
                s.log.clear();
                s.sticks = Default::default();
                s.dropped = 0;
                s.pauses = 0;
                max_this_second = 0;
            }
            if s.reload {
                return;
            }
        }
        if due > 0 {
            let riding = crate::retail_audio::publish(session);
            crate::session_replay::record_riding(riding);
        }
        if second.elapsed() >= Duration::from_secs(1) {
            let mut s = shared.lock().unwrap();
            s.tick_us_max = max_this_second;
            s.animation_bytes = session.animation_bytes();
            s.state = session.state_name();
            s.animation = session.animation_name().unwrap_or("-").to_owned();
            s.speed = session.board().1;
            drop(s);
            max_this_second = 0;
            second = Instant::now();
        } else if due > 0 {
            let mut s = shared.lock().unwrap();
            s.state = session.state_name();
            s.animation = session.animation_name().unwrap_or("-").to_owned();
            s.speed = session.board().1;
        }
        if let Some(error) = session.error() {
            let saved = save_recording(&recording, error, difficulty, map);
            let mut s = shared.lock().unwrap();
            s.error = Some(format!("reloading after tick {}: {}", session.ticks(), truncate(error, 40)));
            s.saved = saved;
            s.recovering = true;
            drop(s);
            crate::retail_audio::silence();
            return; // the sim thread reloads the session
        }
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
    }
}

/// Queues one tick's HUD input. If the main thread has fallen behind, the oldest entry
/// is merged into the next one (its pulses are kept; the movie sees the newer values).
fn push_hud(s: &mut Shared, feed: skate_sim::live::HudFeed) {
    const LIMIT: usize = 8;
    if s.hud_feeds.len() >= LIMIT {
        if let Some(old) = s.hud_feeds.pop_front() {
            if let Some(next) = s.hud_feeds.front_mut() {
                next.new_trick |= old.new_trick;
                next.modified_trick |= old.modified_trick;
                next.close_tricks |= old.close_tricks;
            }
        }
    }
    s.hud_latest = feed.clone();
    s.hud_feeds.push_back(feed);
}

/// Appends a tick's pair to the ring, reusing the oldest entry's bone buffers.
pub(crate) fn publish(
    ring: &mut std::collections::VecDeque<Published>,
    due: Instant,
    period: Duration,
    previous: &Snapshot,
    current: &Snapshot,
    cars: &[skate_sim::CarPose],
    npcs: &[skate_sim::NpcPose],
    peds: &[skate_sim::PedPose],
    hand_props: &[skate_sim::HandPropPose],
) {
    let entry = if ring.len() >= RING {
        let mut e = ring.pop_front().unwrap();
        e.due = due;
        e.period = period;
        copy_snapshot(&mut e.previous, previous);
        copy_snapshot(&mut e.current, current);
        e.cars.clear();
        e.cars.extend_from_slice(cars);
        // The NPC poses into the entry's own buffers (no allocation once the ring is warm).
        e.npcs.truncate(npcs.len());
        for (i, n) in npcs.iter().enumerate() {
            match e.npcs.get_mut(i) {
                Some(d) => {
                    d.serial = n.serial;
                    d.alpha = n.alpha;
                    copy_snapshot(&mut d.previous, &n.previous);
                    copy_snapshot(&mut d.current, &n.current);
                }
                None => e.npcs.push(n.clone()),
            }
        }
        copy_peds(&mut e.peds, peds);
        e.hand_props.clear();
        e.hand_props.extend_from_slice(hand_props);
        e
    } else {
        Published {
            due,
            period,
            previous: previous.clone(),
            current: current.clone(),
            cars: cars.to_vec(),
            npcs: npcs.to_vec(),
            peds: peds.to_vec(),
            hand_props: hand_props.to_vec(),
        }
    };
    ring.push_back(entry);
}

/// The ped poses into an existing list's buffers (no allocation once warm).
pub(crate) fn copy_peds(to: &mut Vec<skate_sim::PedPose>, from: &[skate_sim::PedPose]) {
    to.truncate(from.len());
    for (i, p) in from.iter().enumerate() {
        match to.get_mut(i) {
            Some(d) => {
                (d.serial, d.model, d.tint_a, d.tint_b, d.alpha, d.lod, d.hand) = (p.serial, p.model, p.tint_a, p.tint_b, p.alpha, p.lod, p.hand);
                (d.previous.0, d.previous.1) = (p.previous.0, p.previous.1);
                d.previous.2.clone_from(&p.previous.2);
                (d.current.0, d.current.1) = (p.current.0, p.current.1);
                d.current.2.clone_from(&p.current.2);
            }
            None => to.push(p.clone()),
        }
    }
}

pub(crate) fn copy_snapshot(to: &mut Snapshot, from: &Snapshot) {
    to.root = from.root;
    to.camera = from.camera;
    to.fov = from.fov;
    to.bones.clone_from(&from.bones);
}

/// Writes the session's inputs as a sim-oracle `.input` file; returns its path.
fn save_recording(recording: &[[i32; 7]], error: &str, difficulty: &str, map: &str) -> Option<String> {
    let _ = std::fs::create_dir_all(RECORDINGS);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = format!("{RECORDINGS}/lab-{stamp}.input");
    let mut text = format!(
        "# difficulty={difficulty}\n# map={map}\n# scenario=skate vita session, {} ticks\n# error={}\n",
        recording.len(),
        error.lines().next().unwrap_or("").chars().take(200).collect::<String>()
    );
    for r in recording {
        text += &format!("{} {} {} {} {} {} {}\n", r[0], r[1], r[2], r[3], r[4], r[5], r[6]);
    }
    match std::fs::write(&path, text) {
        Ok(()) => {
            println!("recording saved: {path} ({} ticks)", recording.len());
            Some(path)
        }
        Err(e) => {
            println!("recording not saved ({path}): {e}");
            None
        }
    }
}

pub(crate) fn draw(g: &mut Gfx, s: &Shared) {
    g.text(16.0, 10.0, 2.0, WHITE, "Stick calibration");
    // Sticks.
    let raw = s.sample.map(|x| x.pad).unwrap_or_default();
    for (i, (name, rawxy)) in [("LEFT", [raw.lx, raw.ly]), ("RIGHT", [raw.rx, raw.ry])].iter().enumerate() {
        let cx = 120.0 + i as f32 * 230.0;
        let cy = 140.0;
        let r = 90.0;
        g.circle([cx, cy], r, 1.5, GREY);
        g.circle([cx, cy], r * 0.95, 1.0, [0.6, 0.2, 0.2, 1.0]);
        g.circle([cx, cy], r * 0.25, 1.0, DIM);
        let stats = &s.sticks[i];
        for k in 0..SECTORS {
            let a0 = k as f32 / SECTORS as f32 * core::f32::consts::TAU;
            let a1 = (k + 1) as f32 / SECTORS as f32 * core::f32::consts::TAU;
            let m = stats.sector_max[k] * r;
            // Screen y is down; converted y is up.
            g.line([cx + m * a0.cos(), cy - m * a0.sin()], [cx + m * a1.cos(), cy - m * a1.sin()], 2.0, YELLOW);
        }
        let rx = (rawxy[0] as f32 - 128.0) / 128.0 * r;
        let ry = (rawxy[1] as f32 - 128.0) / 128.0 * r;
        g.rect(cx + rx - 3.0, cy + ry - 3.0, 6.0, 6.0, GREY);
        let c = s.converted[i];
        g.rect(cx + c[0] * r - 4.0, cy - c[1] * r - 4.0, 8.0, 8.0, GREEN);
        let weakest = stats.weakest();
        g.text(cx - 90.0, 240.0, 1.0, WHITE, &format!("{name} raw {:3} {:3}", rawxy[0], rawxy[1]));
        g.text(cx - 90.0, 252.0, 1.0, WHITE, &format!("conv {:+.2} {:+.2}", c[0], c[1]));
        g.text(cx - 90.0, 264.0, 1.0, GREY, &format!("x {:3}..{:3} y {:3}..{:3}", stats.raw_min[0], stats.raw_max[0], stats.raw_min[1], stats.raw_max[1]));
        let (label, colour) = if weakest.is_finite() && weakest >= 0.95 {
            ("OK", GREEN)
        } else {
            ("low", RED)
        };
        let weakest = if weakest.is_finite() { weakest } else { 0.0 };
        g.text(cx - 90.0, 276.0, 1.0, colour, &format!("min dir {weakest:.3} {label}"));
    }
    // Touch panels and the mapped buttons.
    let panel = |g: &mut Gfx, x: f32, y: f32, name: &str, area: Option<controls::Area>, touches: &[controls::Touch]| {
        let (w, h) = (192.0, 100.0);
        g.text(x, y - 12.0, 1.0, GREY, name);
        g.rect(x, y, w, h, [0.12, 0.13, 0.16, 1.0]);
        let band = controls::EDGE_DEAD_BAND;
        g.rect(x + w * band, y + h * band, w * (1.0 - 2.0 * band), h * (1.0 - 2.0 * band), [0.17, 0.19, 0.23, 1.0]);
        g.line([x + w * 0.5, y], [x + w * 0.5, y + h], 1.0, DIM);
        if let Some(a) = area {
            for t in touches {
                let u = (f32::from(t.x) - f32::from(a.min_x)) / (f32::from(a.max_x) - f32::from(a.min_x)).max(1.0);
                let v = (f32::from(t.y) - f32::from(a.min_y)) / (f32::from(a.max_y) - f32::from(a.min_y)).max(1.0);
                g.rect(x + u * w - 4.0, y + v * h - 4.0, 8.0, 8.0, CYAN);
            }
        }
    };
    let (front, rear) = s.sample.map_or((Vec::new(), Vec::new()), |x| (x.front().to_vec(), x.rear().to_vec()));
    panel(g, 20.0, 310.0, "front touch: L3 | R3", s.front_area, &front);
    panel(g, 240.0, 310.0, "rear touch: LB | RB", s.rear_area, &rear);
    let names = [
        (xbox::A, "A"), (xbox::B, "B"), (xbox::X, "X"), (xbox::Y, "Y"),
        (xbox::LEFT_SHOULDER, "LB"), (xbox::RIGHT_SHOULDER, "RB"),
        (xbox::LEFT_THUMB, "L3"), (xbox::RIGHT_THUMB, "R3"),
        (xbox::DPAD_UP, "Up"), (xbox::DPAD_DOWN, "Down"), (xbox::DPAD_LEFT, "Left"), (xbox::DPAD_RIGHT, "Right"),
        (xbox::START, "Start"), (xbox::BACK, "Back"),
    ];
    let mut held: Vec<&str> = names.iter().filter(|(bit, _)| s.buttons & bit != 0).map(|(_, n)| *n).collect();
    if s.triggers[0] != 0 {
        held.push("LT");
    }
    if s.triggers[1] != 0 {
        held.push("RT");
    }
    g.text(20.0, 424.0, 1.0, WHITE, &format!("sent: {}", held.join(" ")));
    g.text(20.0, 436.0, 1.0, GREY, &format!(
        "rear touch bumpers {}  d-pad bumpers {}",
        if s.settings.rear_touch_bumpers { "on" } else { "off" },
        if s.settings.dpad_bumpers { "on" } else { "off" }
    ));

    // Simulation and tricks.
    let x = 480.0;
    let status = if s.loading {
        (s.error.as_deref().unwrap_or("loading assets..."), YELLOW)
    } else if let Some(e) = &s.error {
        (e.as_str(), RED)
    } else {
        ("running", GREEN)
    };
    let mut y = 50.0;
    g.text(x, y, 1.0, status.1, &format!("sim: {}", truncate(status.0, 56)));
    y += 14.0;
    g.text(x, y, 1.0, WHITE, &format!("state {}  speed {:.1} m/s", s.state, s.speed));
    y += 12.0;
    g.text(x, y, 1.0, GREY, &format!("anim {}", truncate(&s.animation, 52)));
    y += 12.0;
    g.text(x, y, 1.0, GREY, &format!(
        "tick {}  {:.1} ms (max {:.1})  dropped {}  pauses {}  fpscr {:#x}",
        s.ticks, s.tick_us as f32 / 1000.0, s.tick_us_max as f32 / 1000.0, s.dropped, s.pauses, s.fpscr
    ));
    y += 24.0;
    g.text(x, y, 2.0, WHITE, "recognized");
    y += 22.0;
    for (i, name) in TARGETS.iter().enumerate() {
        let n = s.counts[i];
        let colour = if n >= 20 { GREEN } else { WHITE };
        g.text(x, y, 2.0, colour, &format!("{name:<12}{n:>3}"));
        y += 18.0;
    }
    g.text(x, y, 2.0, GREY, &format!("{:<12}{:>3}", "other", s.other));
    y += 26.0;
    for line in &s.log {
        g.text(x, y, 1.0, GREY, &truncate(line, 58));
        y += 11.0;
    }
    if let Some(path) = &s.saved {
        g.text(16.0, 506.0, 1.0, GREY, &format!("last failure recorded: {path}"));
    }
    g.text(16.0, 520.0, 1.0, DIM, "Turn each stick around its edge: every direction should reach OK.   Select+O reset   Start back");
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
