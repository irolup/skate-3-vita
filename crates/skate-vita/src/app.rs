//! The game on the main thread (core 0): menus over the world, the 3D
//! view of the live simulation with the HUD, the pause menu and stick calibration, at
//! 60 fps.
//!
//! The simulation thread (`lab.rs`, core 1) owns the session: it loads the selected
//! world, steps it with the fixed-step accumulator while no menu is open, and publishes
//! each tick's presentation pair, score and sound cues. Interpolation: a
//! frame at time `t` shows virtual time `v = t - 1.5 periods` (display delay: the pair
//! it blends is complete as long as a tick takes at most 1.5 periods, 25 ms) with
//! skate-game's overstep blend `alpha = (v - due) / period`. If the simulation has not
//! published the next pair in time, the frame holds the last one at `alpha = 1` and
//! counts a hold.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};
use skate_sim::live::{self, Snapshot};
use vita_pack::skater::Skater;
use vita_platform::controls::sce;
use vita_platform::vita;
use vita_render::scene::{Frame, Scene};
use vita_render::ui::*;
use vita_render::{course, math};

use crate::hud::{self, Names};
use crate::lab::{self, Shared};
use crate::menu::{self, Menu, Row};
use crate::packs::WORLDS;
use crate::settings::Settings;
use crate::skin::{self, Binding};

const SKATER_PACK: &str = "ux0:data/SKATEVITA/skater.pak";
/// Scripted builds (benchmark replay, soak) start in play and ignore Start.
const AUTOPLAY: bool = cfg!(any(feature = "replay", feature = "soak"));
const TRICK_NAMES: &str = "vfs:/assets/private/hud/trick-names.txt";
/// The APT texture holding the world behind the pause menu, and the paused frames drawn before it is captured.
const FROZEN_WORLD: &str = "frozen-world";
const FREEZE_AFTER_FRAMES: u32 = 4;
/// The APT texture of the loading screen's background, and how long the loading screen waits at most.
const LOADING_BACKGROUND: &str = "loading-background";
const LOADING_LIMIT: Duration = Duration::from_secs(180);
/// The Trick Guide's 3D demo (vita-bake `trickguide`): read from the card when the guide opens.
const TRICK_GUIDE_PACK: &str = "ux0:data/SKATEVITA/trickguide.pak";

/// Mounts `trickguide.pak` from the card (its `.idx`, written by vita-bake, lists the files); false without it.
fn mount_trick_guide() -> bool {
    let started = Instant::now();
    match crate::packs::mount_on_card(TRICK_GUIDE_PACK, crate::packs::ROOT) {
        Ok(n) => {
            println!("trickguide.pak: {n} files on the card in {:.0} ms", started.elapsed().as_secs_f32() * 1000.0);
            true
        }
        Err(e) => {
            println!("{TRICK_GUIDE_PACK}: {e}; the Trick Guide has no demo");
            false
        }
    }
}

/// The loading screen's background: the LiveArea picture (`static/sce_sys/pic0.png`, 960 x 544), as opaque RGBA
/// with the bottom row first.
fn loading_background() -> Result<Vec<u8>, String> {
    let mut decoder = png::Decoder::new(&include_bytes!("../../../static/sce_sys/pic0.png")[..]);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    if (w, h) != (math::WIDTH as usize, math::HEIGHT as usize) {
        return Err(format!("pic0.png is {w} x {h}, not the screen's size"));
    }
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => return Err(format!("pic0.png: unexpected colour type {other:?}")),
    };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in (0..h).rev() {
        for px in buf[y * w * channels..(y + 1) * w * channels].chunks_exact(channels) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    Ok(rgba)
}

/// The loading circle at the bottom right: a ring whose bright head turns once a second, its tail fading.
/// The whole screen black (under the retail loading screen), in the menus' stage space.
fn draw_black(apt: &mut vita_render::apt::Apt) {
    use skate_ui::fe::apt_scene::{Draw, Fill, Vertex};
    let v = |x: f32, y: f32| Vertex { position: [x, y], uv: [0.0, 0.0] };
    let (x0, y0, x1, y1) = (-1280.0, -720.0, 2560.0, 1440.0);
    apt.draw(&[Draw { texture: String::new(), vertices: vec![v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y0), v(x1, y1), v(x0, y1)], multiply: [0.0, 0.0, 0.0, 1.0], add: [0.0; 4], fill: Fill::Solid }]);
}

fn draw_spinner(g: &mut Gfx, seconds: f32) {
    const SEGMENTS: usize = 24;
    let (centre, radius) = ([912.0, 496.0], 16.0);
    let head = seconds * core::f32::consts::TAU;
    for i in 0..SEGMENTS {
        let a = head - i as f32 / SEGMENTS as f32 * core::f32::consts::TAU;
        let b = a - core::f32::consts::TAU / SEGMENTS as f32;
        let fade = 1.0 - i as f32 / SEGMENTS as f32;
        g.line(
            [centre[0] + radius * a.cos(), centre[1] + radius * a.sin()],
            [centre[0] + radius * b.cos(), centre[1] + radius * b.sin()],
            4.0,
            [1.0, 1.0, 1.0, 0.15 + 0.85 * fade * fade],
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// The loading screen at startup: the world and every UI loader, then straight into the game.
    Loading,
    Main,
    Pause,
    Playing,
    /// Sub-screens remember whether they were opened from the pause menu.
    Settings { paused: bool },
    Controls { paused: bool },
    Calibration { paused: bool },
    /// Replay of the recorded ticks; the simulation is held.
    Replay,
}

impl Screen {
    fn name(self) -> &'static str {
        match self {
            Screen::Loading => "loading",
            Screen::Main => "main menu",
            Screen::Pause => "pause menu",
            Screen::Playing => "playing",
            Screen::Settings { .. } => "settings",
            Screen::Controls { .. } => "controls",
            Screen::Calibration { .. } => "calibration",
            Screen::Replay => "replay",
        }
    }
    /// Where Back leads.
    fn parent(self) -> Screen {
        match self {
            Screen::Settings { paused } | Screen::Controls { paused } | Screen::Calibration { paused } => {
                if paused { Screen::Pause } else { Screen::Main }
            }
            Screen::Pause => Screen::Playing,
            Screen::Replay => Screen::Pause,
            other => other,
        }
    }
    /// The simulation steps on these screens.
    fn live(self) -> bool {
        matches!(self, Screen::Playing | Screen::Calibration { .. })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Skate,
    Resume,
    Respawn,
    Park,
    Difficulty,
    Physics,
    Traffic,
    AiSkaters,
    Pedestrians,
    Controls,
    Calibration,
    Settings,
    MainMenu,
    Quit,
    RearTouch,
    DpadBumpers,
    FrontTouch,
    Volume,
    DebugHud,
    Audio,
    ManualMeter,
    TrickAnalyzer,
    MenuStyle,
    Replay,
    Back,
}

/// The Replay Editor choice of the plain pause menu (its row label reads it).
static REPLAY_CHOICE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Menu state kept across frames.
struct Menus {
    /// Cursor per menu: main, pause, settings.
    cursors: [usize; 3],
    /// The park selected in the menus (loaded on Skate / Park).
    world: usize,
    /// The Replay Editor choice of the plain pause menu, and a choice to open (0 the instant
    /// replay, k the k-th saved replay).
    replay: usize,
    replay_request: Option<usize>,
}

pub fn run() -> ! {
    let clocks = vita::set_clocks();
    println!("skate vita {}; clocks {clocks:?} MHz", env!("CARGO_PKG_VERSION"));
    // Before anything fragments the heap: the streamed districts' collision buffers.
    crate::alloc::tag_thread("main");
    crate::alloc::stage("boot");
    skate_sim::collision_window::reserve_buffers(crate::packs::WINDOW_BUFFERS.0, crate::packs::WINDOW_BUFFERS.1);
    crate::alloc::stage("window buffers reserved");
    let mut settings = Settings::load();
    let world = usize::from(settings.world).min(WORLDS.len() - 1);
    println!("settings: {settings:?}");
    let shared = Arc::new(Mutex::new(Shared::default()));
    {
        let mut s = shared.lock().unwrap();
        (s.world, s.world_loaded) = (world, world);
        s.difficulty = settings.difficulty;
        s.solver_iterations = settings.solver_iterations();
        s.living = settings.living();
        s.settings = settings.controls;
        s.paused = !AUTOPLAY;
    }
    {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("sim".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || lab::sim_thread(&shared))
            .expect("spawn the simulation thread");
    }
    vita_render::init();
    let mut gfx = Gfx::new();
    let mut apt = vita_render::apt::Apt::new();
    // The Vita's button icons for the retail screens' glyph slots (64 x 64 each).
    for icon in crate::button_icons::Icon::ALL {
        apt.add_texture(icon.key(), crate::button_icons::SIZE, crate::button_icons::SIZE, &crate::button_icons::pixels(icon));
    }
    let mut retail = crate::retail_hud::Hud::Idle;
    let mut marker: Option<crate::retail_hud::Marker> = None;
    let mut marker_tried = false;
    let mut meter: Option<crate::retail_hud::Meter> = None;
    // The HUD minimap (retail_hud::Minimap) and this frame's player, camera forward and called skaters for it.
    let mut minimap: Option<crate::retail_hud::Minimap> = None;
    let mut minimap_view: Option<(Vec3, Vec3)> = None;
    let mut called_positions: Vec<Vec3> = Vec::new();
    let mut analyzer: Option<crate::retail_hud::Analyzer> = None;
    // R2: the retail front end on upstream's runtime, the main and pause menu (the plain menus without it).
    let mut fe_menus = crate::fe_menus::FeMenus::Idle;
    // The screen of the previous frame: entering the pause screen opens the front end.
    let mut fe_prev_screen = Screen::Main;
    // The world behind the pause menu, captured once it stands still (frames shown on the pause screen so far).
    let mut paused_frames = 0u32;
    // The pause screen asked for the front end before it was ready: it opens once it is.
    let mut fe_waiting = false;
    let mut fe_input: Option<vita::Input> = None;
    // Retail teleporters (ui.pak), and the game's buttons last frame (D-pad Up asks).
    let mut teleporters = crate::teleporters::Teleporters::default();
    let mut teleporters_loaded = false;
    // Their floating icons (teleporter_icons.rs): loaded with them, uploaded to the scene once; this frame's anchors and
    // icons.
    let mut teleporter_icons: Option<crate::teleporter_icons::Icons> = None;
    let (mut icon_anchors, mut icons, mut icon_instances) = (Vec::new(), Vec::new(), Vec::<vita_render::scene::IconInstance>::new());
    let mut icon_world: Option<usize> = None;
    // The zombie cheat's colour grade (colour_grade.rs), loaded the first time the cheat is on.
    let mut zombie_grade: Option<Option<vita_render::scene::Grade>> = None;
    let mut teleporter_buttons: u16 = 0;
    let started = Instant::now();
    let mut scene = Scene::new(&course::mesh(&live::course_surfaces()));
    let mut streamer = crate::district_stream::Streamer::default();
    let (joints, pack_error) = match std::fs::read(SKATER_PACK)
        .map_err(|e| format!("{SKATER_PACK}: {e}"))
        .and_then(|bytes| Skater::from_pack(&bytes))
    {
        Ok(skater) => {
            scene.load_skater(&skater);
            (Some(skater.joints), None)
        }
        Err(e) => {
            println!("skater pack not loaded: {e}");
            (None, Some(e))
        }
    };
    crate::audio::FEED.lock().unwrap().params.volume = volume(&settings);
    // The soak proves the retail audio over every world, whatever the saved setting says.
    #[cfg(feature = "soak")]
    {
        settings.retail_audio = true;
    }
    crate::audio::start(settings.retail_audio);
    println!("renderer ready in {:.1} s", started.elapsed().as_secs_f32());
    crate::alloc::stage("renderer ready, audio started");

    let mut screen = if AUTOPLAY { Screen::Playing } else { Screen::Loading };
    // The loading screen's background, rows bottom first like a frame read back (`Apt::draw_screen`).
    match loading_background() {
        Ok(rgba) => apt.add_texture(LOADING_BACKGROUND, math::WIDTH as u32, math::HEIGHT as u32, &rgba),
        Err(e) => println!("loading screen background: {e}"),
    }
    let mut menus = Menus { cursors: [0; 3], world, replay: 0, replay_request: None };
    let mut buttons = menu::Buttons::default();
    let mut names: Option<Names> = None;
    let mut binding: Option<Binding> = None;
    // The simulation rig's bone names (the Trick Guide demo's clips are matched to them).
    let mut rig_names: Vec<String> = Vec::new();
    // The Trick Guide's 3D demo while its screen is open (its set in the scene's demo slot), its skin palette, and
    // whether trickguide.pak is mounted (None: not tried, Some(false): missing).
    let mut trick_demo: Option<crate::trick_demo::Demo> = None;
    let mut demo_palette: Vec<Mat4> = Vec::new();
    let mut demo_pack: Option<bool> = None;
    let mut binding_error: Option<String> = None;
    let (mut a, mut b) = (empty_snapshot(), empty_snapshot());
    let mut palette = Vec::new();
    // The props streamed in and their templates in the props render.
    let mut prop_poses: Vec<skate_sim::live::PropPose> = Vec::new();
    let mut prop_instances: Vec<vita_render::scene::PropInstance> = Vec::new();
    let mut prop_templates: std::collections::HashMap<(u32, u32), u16> = std::collections::HashMap::new();
    let mut props_logged = true;
    // The traffic cars of the shown tick and what the scene draws.
    let mut car_poses: Vec<skate_sim::CarPose> = Vec::new();
    let mut car_instances: Vec<vita_render::scene::CarInstance> = Vec::new();
    // The NPC skaters of the shown tick and their palettes.
    let mut npc_poses: Vec<skate_sim::NpcPose> = Vec::new();
    // The called skaters' trails (called_trails.rs): their NPC serials as of the last tick, the trails to draw and the
    // seconds of play this frame.
    let mut called_trails = crate::called_trails::CalledTrails::default();
    // A world load was showing last frame (the retail loading screen starts on a load's first frame only).
    let mut was_loading = false;
    let (mut called_serials, mut trail_instances, mut trail_dt) = (Vec::<u32>::new(), Vec::<vita_render::scene::TrailInstance>::new(), 0.0);
    let mut npc_palettes: Vec<(Vec<Mat4>, f32, Vec3)> = Vec::new();
    // The peds of the shown tick, each model's binding to the ped rig, their palettes.
    let mut ped_poses: Vec<skate_sim::PedPose> = Vec::new();
    let mut ped_bindings: Vec<Option<skin::PedBinding>> = Vec::new();
    let mut ped_palettes: Vec<Vec<Mat4>> = Vec::new();
    // The released hand props of the shown tick, the rig's hand bone, this frame's prop frames.
    let mut hand_poses: Vec<skate_sim::HandPropPose> = Vec::new();
    let mut hand_bone: Option<usize> = None;
    let mut hand_frames: Vec<(u16, Mat4, f32)> = Vec::new();
    let mut playback: Option<crate::session_replay::Playback> = None;
    let mut replay_note = String::new();
    let mut replay_glyphs: Vec<(String, [f32; 4])> = Vec::new();
    let mut help_open = false;
    // The retail replay screen on the menu stage (FE 12 / FE 13) takes the replay (else P1's own screens), and the
    // help's page turns asked since the replay opened.
    let mut retail_replay = false;
    let mut help_turns: u32 = 0;
    // The Finished menu is up as the menu stage's popup (it takes the pad; the replay gets none).
    let mut finish_popup = false;
    // The replay camera's last accepted position (retail's constraint state), and the seek count it belongs to.
    let mut camera_guard: Option<(glam::Vec3, u64, Option<crate::replay_camera::Kind>)> = None;
    let mut replay_origin = Screen::Playing;
    let mut pending_replay: Option<(String, usize)> = None;
    let mut replay_input: Option<vita::Input> = None;
    let mut replay_prev: u16 = u16::MAX;
    let mut replay_finish: Option<usize> = None;
    let mut replay_band: Option<(f32, f32, bool)> = None;
    let mut replay_zones: Vec<(f32, f32, crate::replay_camera::Kind, bool)> = Vec::new();
    let mut photo_note: Option<(String, Instant)> = None;
    let mut replay_markers: (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    let mut stats = Stats::default();
    let mut line_peak = 0.0f32;
    let mut quit = false;
    let mut last_frame = Instant::now();
    // The world whose textures are still uploading: (world, since, upload time so far).
    let mut uploading: Option<(usize, Instant, Duration)> = None;
    while !quit {
        vita::keep_awake();
        let frame_start = Instant::now();
        if frame_start - last_frame > Duration::from_millis(250) {
            println!("SUSPEND main thread: {} ms gap on the {} screen", (frame_start - last_frame).as_millis(), screen.name());
        }
        let prev_frame = last_frame;
        last_frame = frame_start;
        let pad = vita::pad();
        let pressed = buttons.pressed(pad.buttons);
        let frame_dt = (frame_start - prev_frame).as_secs_f32().min(0.1);
        let mut view: Option<f32> = None;
        let mut photo_request = false;
        let marker_view;
        let manual_view;
        let score_view;
        let boot_note;
        let loading_view;
        let hud_note;
        let pending_park;
        let pending_props;
        let pending_cars;
        let pending_peds;
        let pending_hand_props;
        let info;
        {
            let mut s = shared.lock().unwrap();
            if s.quit {
                break;
            }
            pending_park = s.park_render.take();
            // No trails in a replay (its NPCs are the recording's); they stand still while paused.
            called_serials.clear();
            if screen != Screen::Replay {
                called_serials.extend_from_slice(&s.called_serials);
            }
            trail_dt = if screen == Screen::Playing && !s.paused { frame_dt } else { 0.0 };
            pending_props = s.props_render.take();
            pending_cars = s.cars_render.take();
            pending_peds = s.peds_render.take();
            pending_hand_props = s.hand_props_render.take();
            prop_poses.clear();
            prop_poses.extend_from_slice(&s.props);
            // The UI loaders run one after another, and only once the first park has loaded:
            // they allocate heavily and slow each other and the park load many times over
            // when they overlap (device log: the park load took 45 s instead of 8 s). The
            // trick display, the marker overlay, the replay screens, then the retail front end (`fe_menus`, the main
            // and pause menu; the plain menus stand in when it is missing).
            if crate::packs::MOUNTED.load(std::sync::atomic::Ordering::Relaxed) && s.bone_names.is_some() && !s.loading {
                retail.start();
                // The marker overlay needs ui.pak too (mounted by now).
                if !marker_tried && !retail.booting() && crate::packs::UI.load(std::sync::atomic::Ordering::Relaxed) {
                    marker_tried = true;
                    match crate::retail_hud::Marker::load(&mut apt) {
                        Ok(m) => marker = Some(m),
                        Err(e) => println!("session marker overlay off: {e}"),
                    }
                    if crate::retail_hud::screen_present(crate::retail_hud::Meter::FILE) {
                        match crate::retail_hud::Meter::load(&mut apt) {
                            Ok(m) => meter = Some(m),
                            Err(e) => println!("manual meter off: {e}"),
                        }
                    }
                    if crate::retail_hud::screen_present(crate::retail_hud::Minimap::FILE) {
                        match crate::retail_hud::Minimap::load(&mut apt) {
                            Ok(m) => minimap = Some(m),
                            Err(e) => println!("minimap off: {e}"),
                        }
                    }
                    if crate::retail_hud::screen_present(crate::retail_hud::Analyzer::FILE) {
                        match crate::retail_hud::Analyzer::load(&mut apt) {
                            Ok(a) => analyzer = Some(a),
                            Err(e) => println!("trick analyzer off: {e}"),
                        }
                    }
                }
            }
            if names.is_none() && s.bone_names.is_some() {
                // sim.pak is mounted once the first session has loaded.
                let loaded = Names::parse(&skate_data::vfs::read_to_string(std::path::Path::new(TRICK_NAMES)).unwrap_or_default());
                println!("trick names: {}", loaded.len());
                names = Some(loaded);
            }
            if binding.is_none() && binding_error.is_none() {
                if let (Some(joints), Some(bone_names)) = (&joints, &s.bone_names) {
                    match Binding::new(joints, bone_names) {
                        Ok(bound) => {
                            println!("skater bound: {} joints to the stock rig", bound.len());
                            binding = Some(bound);
                            rig_names = bone_names.clone();
                        }
                        Err(e) => {
                            println!("skater binding failed: {e}");
                            binding_error = Some(e);
                        }
                    }
                }
            }

            // Soak: the pause menu for 3 s of every 2 minutes.
            #[cfg(feature = "soak")]
            {
                screen = match (crate::soak::pause_now(started.elapsed()), screen) {
                    (true, Screen::Playing) => Screen::Pause,
                    (false, Screen::Pause) => Screen::Playing,
                    (_, other) => other,
                };
                let recorded = crate::session_replay::RECORDER.lock().unwrap().len() > 1;
                screen = match (crate::soak::replay_now(started.elapsed()), screen) {
                    (true, Screen::Playing) if recorded => Screen::Replay,
                    (false, Screen::Replay) => Screen::Playing,
                    (_, other) => other,
                };
            }
            // Playing from the main menu waits until the UI loaders are done.
            let boot = retail.booting() || (!marker_tried && crate::packs::UI.load(std::sync::atomic::Ordering::Relaxed));
            let screen_before = screen;
            // While the R2 front end is open it takes the pad (below); Triangle alone in the pause menu opens it.
            let fe_screen = |screen: Screen| matches!(screen, Screen::Main | Screen::Pause);
            let fe_shown = fe_screen(screen) && fe_menus.shown();
            let mut changed = if fe_shown { false } else { handle_input(&mut screen, &mut menus, &mut settings, &mut s, pad.buttons, pressed, &mut quit, boot) };
            // Entering the pause screen (from playing, the controls page, the replay...) opens the front end, which
            // waits with a note while it loads; the plain menus style keeps the plain pause menu.
            if fe_screen(screen) && fe_prev_screen != screen && settings.retail_menus && !AUTOPLAY {
                fe_waiting = fe_menus.loading() || fe_menus.ready();
            }
            fe_prev_screen = screen;
            // Leaving the menus (or the front end failing) cancels the wait.
            fe_waiting &= fe_screen(screen) && (fe_menus.loading() || fe_menus.ready());
            if fe_waiting && fe_menus.ready() {
                fe_waiting = false;
                fe_menus.open(&settings, &crate::session_replay::choices(), screen == Screen::Main);
            }
            if fe_screen(screen) && fe_menus.shown() && !settings.retail_menus {
                fe_menus.close_now();
            }
            if fe_screen(screen) && fe_menus.shown() {
                let paused = screen == Screen::Pause;
                let input = fe_input.get_or_insert_with(vita::Input::new);
                let sample = input.poll();
                let mut mapping = settings.controls;
                (mapping.rear_touch_bumpers, mapping.front_touch_thumbs, mapping.dpad_bumpers) = (true, true, false);
                let values = skate_core::input::xbox::convert(&input.xbox(&sample, &mapping), 0);
                let update = fe_menus.update(&values, frame_dt, &mut settings, &s.called_skaters);
                changed |= update.settings_changed;
                s.skater_calls.extend(update.skater_calls);
                let mut left = false;
                for choice in update.choices {
                    use crate::vita_menus::Choice;
                    left = true;
                    match choice {
                        Choice::Skate => {
                            fe_menus.close();
                            changed |= start_playing(&mut screen, &menus, &mut settings, &mut s);
                        }
                        Choice::Respawn => {
                            s.respawn = true;
                            fe_menus.close();
                            screen = Screen::Playing;
                        }
                        Choice::World(w) => {
                            menus.world = w.min(WORLDS.len() - 1);
                            fe_menus.close();
                            changed |= start_playing(&mut screen, &menus, &mut settings, &mut s);
                        }
                        Choice::MainMenu => {
                            fe_menus.close_now();
                            screen = Screen::Main;
                        }
                        Choice::Quit => quit = true,
                        Choice::Controls => {
                            fe_menus.close_now();
                            screen = Screen::Controls { paused };
                        }
                        Choice::Calibration => {
                            fe_menus.close_now();
                            screen = Screen::Calibration { paused };
                        }
                        Choice::Replay(k) => {
                            fe_menus.close_now();
                            menus.replay_request = Some(k);
                        }
                        Choice::TrickAnalyzer | Choice::ManualMeter => {
                            if choice == Choice::TrickAnalyzer {
                                settings.trick_analyzer = !settings.trick_analyzer;
                            } else {
                                settings.manual_meter = !settings.manual_meter;
                            }
                            fe_menus.set_facts(&settings);
                            changed = true;
                            left = false;
                        }
                    }
                }
                // A Challenge Map location: here, or once its world has loaded (the sim thread waits for the load).
                for (map, matrix) in update.teleports {
                    left = true;
                    fe_menus.close();
                    match WORLDS.iter().position(|w| *w == map) {
                        Some(w) => {
                            s.teleport = Some(matrix);
                            menus.world = w;
                            changed |= start_playing(&mut screen, &menus, &mut settings, &mut s);
                        }
                        None => println!("challenge map: no world {map} on the Vita"),
                    }
                }
                if !update.open && !left {
                    menus.world = s.world_loaded;
                    screen = Screen::Playing;
                }
                fe_prev_screen = screen;
            }
            // Retail teleporters (teleporters.rs): inside a volume the action overlay prompts, D-pad Up (not with LB, the
            // session marker's) asks to confirm, Yes moves the skater (here, or once its world has loaded).
            if !teleporters_loaded && crate::packs::UI.load(std::sync::atomic::Ordering::Relaxed) {
                teleporters_loaded = true;
                let read = |p: &str| skate_data::vfs::read(&std::path::Path::new(crate::packs::ROOT).join(p)).map_err(|e| e.to_string());
                teleporters = crate::teleporters::Teleporters::load(&read);
                teleporter_icons = teleporters.icon_key().and_then(|key| crate::teleporter_icons::Icons::load(&read, key));
                if let Some(i) = &teleporter_icons {
                    let texture = |(w, h, rgba): &(u32, u32, Vec<u8>)| vita_pack::texture::Texture {
                        width: *w as u16,
                        height: *h as u16,
                        format: vita_pack::texture::Format::Rgba8,
                        mips: vec![rgba.clone()],
                    };
                    scene.load_icons(&i.vertices, &i.indices, &texture(&i.front), &texture(&i.background));
                }
            }
            {
                let input = fe_input.get_or_insert_with(vita::Input::new);
                let sample = input.poll();
                let x = input.xbox(&sample, &settings.controls);
                let values = skate_core::input::xbox::convert(&x, 0);
                use vita_platform::controls::xbox;
                let up = x.buttons & !teleporter_buttons & xbox::DPAD_UP != 0 && x.buttons & xbox::LEFT_SHOULDER == 0;
                teleporter_buttons = x.buttons;
                let playing = screen == Screen::Playing && !AUTOPLAY;
                let world = (playing && !s.loading && !s.reload).then(|| WORLDS[s.world_loaded]);
                icon_world = (!s.loading && !s.reload).then_some(s.world_loaded);
                if let Some(e) = s.frames.back() {
                    teleporters.track(world, e.current.root.translation.to_array());
                }
                if !playing && teleporters.asking() {
                    teleporters.cancel();
                }
                let popup = (playing && up && fe_menus.has_signup()).then(|| teleporters.ask()).flatten();
                let prompt = teleporters.prompt().filter(|_| playing).map(str::to_owned);
                if let Some(choice) = fe_menus.hud(prompt, popup, frame_dt, &values) {
                    if let Some(exit) = teleporters.choose(choice) {
                        println!("teleporter: to {} ({})", exit.map, exit.locator);
                        match WORLDS.iter().position(|w| w.eq_ignore_ascii_case(&exit.map)) {
                            Some(w) => {
                                s.teleport = Some(exit.matrix);
                                menus.world = w;
                                changed |= start_playing(&mut screen, &menus, &mut settings, &mut s);
                            }
                            None => println!("teleporter: no world {} on the Vita", exit.map),
                        }
                    }
                }
            }
            // The pause menu's Replay Editor: the instant replay, or a saved replay (its park loads first).
            if let Some(choice) = menus.replay_request.take() {
                if choice == 0 {
                    if crate::session_replay::RECORDER.lock().unwrap().len() > 1 {
                        screen = Screen::Replay;
                    }
                } else if let Some(saved) = crate::session_replay::SAVED.lock().unwrap().get(choice - 1) {
                    match WORLDS.iter().position(|w| *w == saved.world) {
                        Some(w) => {
                            if w != s.world_loaded {
                                (s.world, s.reload, menus.world, settings.world) = (w, true, w, w as u8);
                            }
                            pending_replay = Some((saved.name.clone(), w));
                        }
                        None => println!("replay {}: unknown world {}", saved.name, saved.world),
                    }
                }
            }
            if let Some((name, w)) = &pending_replay {
                // Once the park is loaded and its session runs (it empties the recorder first).
                if s.world_loaded == *w && !s.loading && !s.reload && !s.frames.is_empty() {
                    match crate::session_replay::load(name) {
                        Ok((_, markers, clips)) => {
                            playback = crate::session_replay::Playback::loaded(&crate::session_replay::RECORDER.lock().unwrap(), markers, clips);
                            if playback.is_some() {
                                screen = Screen::Replay;
                                crate::retail_audio::set_replaying(true);
                                replay_prev = u16::MAX;
                            }
                        }
                        Err(e) => {
                            println!("replay {name}: {e}");
                            *crate::session_replay::SAVE_STATUS.lock().unwrap() = format!("{name}: {e}");
                        }
                    }
                    pending_replay = None;
                }
            }
            // After the input (it can change the screen or the style): which menu shows now,
            // and the crossbar opens in the same frame the screen appears.
            if screen == Screen::Replay && screen_before != Screen::Replay {
                replay_origin = screen_before;
            }
            if screen == Screen::Replay && playback.is_none() {
                playback = crate::session_replay::Playback::new(&crate::session_replay::RECORDER.lock().unwrap());
                println!("replay: {} recorded ticks", crate::session_replay::RECORDER.lock().unwrap().len());
                match playback {
                    Some(_) => {
                        crate::retail_audio::set_replaying(true);
                        replay_prev = u16::MAX;
                    }
                    None => screen = Screen::Pause,
                }
            } else if screen != Screen::Replay && playback.is_some() {
                playback = None;
                crate::retail_audio::set_replaying(false);
            }
            let in_menu = matches!(screen, Screen::Main | Screen::Pause);
            if changed {
                s.settings = settings.controls;
                s.difficulty = settings.difficulty;
                s.solver_iterations = settings.solver_iterations();
                let living = settings.living();
                if living != s.living {
                    s.living = living;
                    s.living_changed = true;
                }
                crate::audio::FEED.lock().unwrap().params.volume = volume(&settings);
                crate::audio::set_retail(settings.retail_audio);
                settings.save();
            }
            retail.frame(&mut apt, &mut s, screen == Screen::Playing);
            // The Trick Guide's demo pack (its index only: the files stay on the card), mounted while the loading screen
            // shows so the guide opens without a wait.
            if demo_pack.is_none() && screen == Screen::Loading && crate::packs::MOUNTED.load(std::sync::atomic::Ordering::Relaxed) {
                demo_pack = Some(mount_trick_guide());
            }
            // The R2 front end loads last (the loaders slow each other down when they overlap).
            if fe_menus.idle() && marker_tried {
                fe_menus.start(settings, menu_worlds());
            }
            fe_menus.frame(&mut apt, frame_dt);
            // The simulation is held while the HUD movie of a new session is built.
            let hud_wait = screen == Screen::Playing && retail.building() && !AUTOPLAY;
            // The retail loading screen's close (fade out, close, fade in) runs before play resumes.
            s.paused = !screen.live() || hud_wait || teleporters.asking() || fe_menus.loading_screen();
            s.want_camera_world = screen == Screen::Replay;
            boot_note = in_menu && retail.booting();
            // The loading screen: until the world and every UI loader are ready (the pause menu included), then
            // straight into the game. A world that failed (or a load past the limit) shows the main menu and its
            // message instead.
            if screen == Screen::Loading {
                let world_ready = !s.loading && !s.reload && s.bone_names.is_some() && !s.frames.is_empty() && uploading.is_none();
                let ui_ready = !retail.booting() && marker_tried && !fe_menus.loading();
                if world_ready && ui_ready {
                    println!("loading screen: ready in {:.1} s", started.elapsed().as_secs_f32());
                    screen = Screen::Playing;
                } else if (s.error.is_some() && !s.loading) || started.elapsed() > LOADING_LIMIT {
                    println!("loading screen: gave up after {:.1} s (error {:?})", started.elapsed().as_secs_f32(), s.error);
                    screen = Screen::Main;
                }
            }
            // A world loading (startup, or another park or district) shows the loading screen.
            loading_view = screen == Screen::Loading || (screen == Screen::Playing && (s.loading || s.reload) && !AUTOPLAY);
            fe_menus.set_loading(loading_view, loading_view && !was_loading);
            was_loading = loading_view;
            hud_note = hud_wait;
            marker_view = s.marker;
            manual_view = s.manual;
            score_view = s.score.clone();

            if matches!(screen, Screen::Calibration { .. }) {
                lab::draw(&mut gfx, &s);
            } else if let (Screen::Replay, Some(pb)) = (screen, playback.as_mut()) {
                let rec = crate::session_replay::RECORDER.lock().unwrap();
                // Skate 3's Xbox layout through the game's own mapping: LT and RT are L and R, LB and RB the
                // rear touch halves, L3 the front touch's left half (the D-pad stays a D-pad).
                let input = replay_input.get_or_insert_with(vita::Input::new);
                let sample = input.poll();
                let mut mapping = settings.controls;
                (mapping.rear_touch_bumpers, mapping.front_touch_thumbs, mapping.dpad_bumpers) = (true, true, false);
                let x = input.xbox(&sample, &mapping);
                let xpressed = x.buttons & !replay_prev;
                replay_prev = x.buttons;
                let axis = |v: i16| f32::from(v) / 32767.0;
                let controls = crate::session_replay::Controls {
                    held: x.buttons,
                    pressed: xpressed,
                    lt: x.triggers[0] > 128,
                    rt: x.triggers[1] > 128,
                    left: [axis(x.left[0]), axis(x.left[1])],
                    right: [axis(x.right[0]), axis(x.right[1])],
                };
                // The menu stage's pad (the Finished popup), in the same Xbox layout.
                let fe_pad = skate_core::input::xbox::convert(&x, 0);
                let (controls, dt) = if retail_replay && pb.finish.is_some() {
                    // The Finished popup has the pad; the replay holds still under it.
                    (crate::session_replay::Controls { held: 0, pressed: 0, lt: false, rt: false, left: [0.0; 2], right: [0.0; 2] }, 0.0)
                } else if help_open {
                    if xpressed & vita_platform::controls::xbox::A != 0 {
                        help_turns += 1;
                    }
                    if xpressed & (vita_platform::controls::xbox::B | vita_platform::controls::xbox::START) != 0 {
                        help_open = false;
                    }
                    // The replay holds still under the help pages.
                    (crate::session_replay::Controls { held: 0, pressed: 0, lt: false, rt: false, left: [0.0; 2], right: [0.0; 2] }, 0.0)
                } else {
                    // The help pages are the retail screen's (FE 13); without it there is no help.
                    if xpressed & vita_platform::controls::xbox::START != 0 && retail_replay {
                        help_open = true;
                    }
                    (controls, frame_dt)
                };
                match pb.update(&rec, dt, &controls) {
                    // Back to where the replay was opened from (the game, not the pause menu, after a tap of Select).
                    crate::session_replay::Outcome::Exit => screen = replay_origin,
                    crate::session_replay::Outcome::Save => {
                        let world = WORLDS[s.world_loaded];
                        if let Err(e) = crate::session_replay::save(&rec, pb, world) {
                            *crate::session_replay::SAVE_STATUS.lock().unwrap() = format!("Save failed: {e}");
                        }
                        // Saving keeps the menu open, with the outcome under it.
                    }
                    crate::session_replay::Outcome::Frame(step) => {
                        view = Some(rec.pair(pb.pos, &mut a, &mut b));
                        // The world as recorded at the same time (a loaded replay has none: the world stays frozen).
                        if let Some(time) = rec.world_time(pb.pos) {
                            crate::world_replay::fill(time, &mut car_poses, &mut npc_poses, &mut ped_poses, &mut hand_poses, &mut prop_poses);
                        }
                        pb.apply_camera(&rec, &mut a, &mut b);
                        // Retail's replay camera constraint (b449): the shown camera out of walls, where the paused
                        // session lends its collision world (a district's: only its loaded window).
                        match (pb.constrained_kind(), view, s.camera_world.as_ref()) {
                            (Some(kind), Some(alpha), Some(world)) => {
                                use crate::replay_camera::{constrain, CameraCollision, Hit};
                                let eye = a.camera.translation.lerp(b.camera.translation, alpha);
                                // The line of sight starts at the body, not the feet (a sweep from the ground hits it).
                                let anchor = a.root.translation.lerp(b.root.translation, alpha) + glam::Vec3::Y;
                                // A seek, a new camera or a jump of over a metre is a cut: no last position.
                                let old = camera_guard
                                    .filter(|(p, seeks, k)| *seeks == pb.seeks && *k == Some(kind) && p.distance(eye) < 1.0)
                                    .map(|(p, ..)| p);
                                let free = kind == crate::replay_camera::Kind::Free;
                                let c = constrain(old, eye, anchor, free, &CameraCollision::default(), |from, to, r| {
                                    world.sweep(from, to, r).map(|(fraction, normal)| Hit { fraction, normal })
                                });
                                for snap in [&mut a, &mut b] {
                                    snap.camera.translation = c.position;
                                    if c.relook {
                                        snap.camera.rotation = glam::Mat4::look_at_rh(c.position, anchor, glam::Vec3::Y).inverse().to_scale_rotation_translation().1;
                                    }
                                }
                                camera_guard = Some((c.position, pb.seeks, Some(kind)));
                            }
                            _ => camera_guard = None,
                        }
                        let frame = rec.frame(pb.pos as usize);
                        crate::retail_audio::publish_replay(frame.riding, step.steps, frame.camera, frame.cuts.wrapping_add(pb.seeks << 40), step.silenced);
                        retail_replay = fe_menus.replay(Some(pb.screen_feed(&rec, help_open, help_turns)), frame_dt, &fe_pad);
                        if retail_replay {
                            // The Finished menu (B): Upload Video (not on the Vita), Save Replay, Exit; B closes it.
                            // Upload and Save keep it open with the outcome under it, as P1.
                            if pb.finish.is_some() && !finish_popup {
                                fe_menus.replay_popup(&crate::session_replay::FINISH_ITEMS);
                                finish_popup = true;
                            }
                            if let Some(choice) = fe_menus.take_replay_choice() {
                                finish_popup = false;
                                match choice {
                                    0 => *crate::session_replay::SAVE_STATUS.lock().unwrap() = "Uploading is not available on the Vita.".into(),
                                    1 => {
                                        let world = WORLDS[s.world_loaded];
                                        if let Err(e) = crate::session_replay::save(&rec, pb, world) {
                                            *crate::session_replay::SAVE_STATUS.lock().unwrap() = format!("Save failed: {e}");
                                        }
                                    }
                                    2 => screen = replay_origin,
                                    _ => pb.finish = None,
                                }
                            }
                        }
                        replay_finish = pb.finish;
                        replay_zones = pb.zones(&rec);
                        photo_request = std::mem::take(&mut pb.photo);
                        let [_, start, end] = pb.fractions(&rec);
                        replay_band = (!retail_replay && !pb.hidden && !help_open && pb.finish.is_none() && matches!(pb.mode, crate::session_replay::Mode::Scrub | crate::session_replay::Mode::Main)).then_some((start, end, pb.mode == crate::session_replay::Mode::Main));
                        replay_markers = if !retail_replay && pb.mode == crate::session_replay::Mode::Main && !pb.hidden && !help_open && pb.finish.is_none() { pb.marker_fractions(&rec) } else { (Vec::new(), Vec::new()) };
                        replay_note = if retail_replay {
                            String::new()
                        } else {
                            format!(
                                "Replay {:.1} / {:.1} s{}   RB pause  LT/RT wind  A edit  B finish",
                                pb.pos * rec.period,
                                (rec.len() - 1) as f32 * rec.period,
                                if pb.paused { " (paused)" } else { "" },
                            )
                        };
                    }
                }
            } else if let Some((index, alpha, hold)) = pick(&s.frames, Instant::now()) {
                let entry = &s.frames[index];
                lab::copy_snapshot(&mut a, &entry.previous);
                lab::copy_snapshot(&mut b, &entry.current);
                car_poses.clear();
                car_poses.extend_from_slice(&entry.cars);
                lab::copy_peds(&mut ped_poses, &entry.peds);
                hand_poses.clone_from(&entry.hand_props);
                npc_poses.truncate(entry.npcs.len());
                for (i, n) in entry.npcs.iter().enumerate() {
                    match npc_poses.get_mut(i) {
                        Some(d) => {
                            d.serial = n.serial;
                            d.alpha = n.alpha;
                            lab::copy_snapshot(&mut d.previous, &n.previous);
                            lab::copy_snapshot(&mut d.current, &n.current);
                        }
                        None => npc_poses.push(n.clone()),
                    }
                }
                view = Some(alpha);
                if hold && !s.loading && !s.paused {
                    stats.holds += 1;
                    stats.holds_total += 1;
                }
            }
            // The replay ended (Exit, or the screen changed under it): the retail replay screen closes now, before
            // anything draws, so it never shows over the pause menu.
            if screen != Screen::Replay && fe_menus.replay_open() {
                fe_menus.replay(None, frame_dt, &[0.0; 24]);
                retail_replay = false;
                help_turns = 0;
                finish_popup = false;
            }
            if s.score.line <= 0.0 {
                line_peak = 0.0;
            }
            line_peak = line_peak.max(s.score.line_time);
            info = Info {
                world: WORLDS[s.world_loaded],
                state: s.state.clone(),
                speed: s.speed,
                tick_ms: s.tick_us as f32 / 1000.0,
                tick_max_ms: s.tick_us_max as f32 / 1000.0,
                dropped: s.dropped,
                animation_bytes: s.animation_bytes,
            };
            if !matches!(screen, Screen::Calibration { .. }) {
                let overlay = Overlay { screen, menus: &menus, settings: &settings, names: names.as_ref(), line_peak, info: &info, stats: &stats, retail_hud: retail.active(), fe_menu: matches!(screen, Screen::Main | Screen::Pause) && (fe_menus.shown() || (settings.retail_menus && (fe_menus.ready() || fe_menus.loading()))), fe_waiting, loading_view, boot: boot_note, hud_wait: hud_note, replay: &replay_note };
                overlay.draw(&mut gfx, &s);
                draw_glyphs(&mut gfx, &replay_glyphs);
                if screen == Screen::Replay {
                    draw_timeline(&mut gfx, replay_band, &replay_zones, &replay_markers.0, &replay_markers.1);
                    if let Some((text, at)) = &photo_note {
                        if at.elapsed() < Duration::from_millis(2500) {
                            shadow(&mut gfx, 330.0, 470.0, 1.0, WHITE, text);
                        }
                    }
                    if let Some(item) = replay_finish {
                        if retail_replay {
                            // The retail popup is drawn; the outcome of a save goes under it.
                            let status = crate::session_replay::SAVE_STATUS.lock().unwrap().clone();
                            shadow(&mut gfx, 480.0 - width(&status, 1.0) / 2.0, 372.0, 1.0, WHITE, &status);
                        } else {
                            draw_finish_menu(&mut gfx, item);
                        }
                    }
                }
            }
        }

        // A new world's render data, uploaded outside the lock.
        // The world's movable props (template meshes), and the lookup of their templates.
        if let Some(models) = pending_hand_props {
            scene.load_hand_props(&models);
        }
        if let Some((models, rig)) = pending_peds {
            hand_bone = rig.as_ref().and_then(|(names, ..)| names.iter().position(|n| n.eq_ignore_ascii_case("RIGHTHANDPROP")));
            let started = Instant::now();
            ped_bindings = match &rig {
                Some((names, parents, animated)) => models.iter().map(|m| m.as_ref().map(|m| skin::PedBinding::new(&m.joints, names, parents, animated))).collect(),
                None => Vec::new(),
            };
            scene.load_peds(&models);
            if !models.is_empty() {
                println!("peds render: {} models in {:.0} ms", models.iter().flatten().count(), started.elapsed().as_secs_f32() * 1000.0);
            }
        }
        if let Some(cars) = pending_cars {
            let started = Instant::now();
            scene.load_cars(&cars);
            if !cars.is_empty() {
                println!("cars render: {} models in {:.0} ms", cars.iter().flatten().count(), started.elapsed().as_secs_f32() * 1000.0);
            }
        }
        if let Some(props) = pending_props {
            prop_templates.clear();
            match props {
                Some(props) => {
                    for (i, t) in props.templates.iter().enumerate() {
                        prop_templates.insert((t.first_index, t.index_count), i as u16);
                    }
                    let started = Instant::now();
                    let textures = props.render.textures.len();
                    scene.load_props(props);
                    println!("props render: {} templates, {} textures in {:.0} ms", prop_templates.len(), textures, started.elapsed().as_secs_f32() * 1000.0);
                    props_logged = false;
                }
                None => scene.unload_props(),
            }
        }
        if let Some((w, render)) = pending_park {
            called_trails.clear(&mut scene);
            if let Some(m) = &mut minimap {
                m.reset();
            }
            let started = Instant::now();
            match render {
                Some(render) => scene.load_park(render),
                None => scene.unload_park(),
            }
            println!("world {}: buffers ready in {:.0} ms, textures upload over the next frames", WORLDS[w], started.elapsed().as_secs_f32() * 1000.0);
            uploading = Some((w, Instant::now(), started.elapsed()));
        }
        // Park textures: a few ms per frame instead of one long frame.
        if let Some((w, since, spent)) = uploading {
            let step = Instant::now();
            let left = scene.upload_pending(Duration::from_millis(3));
            uploading = Some((w, since, spent + step.elapsed()));
            if left == 0 {
                println!("world {}: textures ready after {:.1} s ({:.0} ms of upload work)", WORLDS[w], since.elapsed().as_secs_f32(), (spent + step.elapsed()).as_secs_f32() * 1000.0);
                uploading = None;
            }
        }
        if matches!(screen, Screen::Calibration { .. }) {
            gfx.present();
        } else {
            // The world stands still behind the pause menu (the simulation is held): after a few frames (the shown
            // pair settles at alpha 1) it is captured once and the capture is drawn instead of the scene (DownTown's
            // scene took about 15 ms of every paused frame).
            let freeze = matches!(screen, Screen::Pause | Screen::Main) && !AUTOPLAY;
            paused_frames = if freeze { paused_frames.saturating_add(1) } else { 0 };
            let frozen = freeze && apt.has_texture(FROZEN_WORLD);
            if !freeze && apt.has_texture(FROZEN_WORLD) {
                apt.remove_texture(FROZEN_WORLD);
            }
            // The Trick Guide's demo: its set loads when the guide opens (the screen is black then) and goes with it.
            let demo_wanted = if matches!(screen, Screen::Pause | Screen::Main) { fe_menus.trick_guide_demo() } else { None };
            let read = |p: &str| skate_data::vfs::read(&std::path::Path::new(crate::packs::ROOT).join(p)).map_err(|e| format!("{p}: {e}"));
            if demo_wanted.is_some() && trick_demo.is_none() && demo_pack != Some(false) {
                if *demo_pack.get_or_insert_with(mount_trick_guide) {
                    let started = Instant::now();
                    let loaded = read(&format!("{}/TrickGuide.map", crate::trick_demo::ROOT))
                        .and_then(|b| vita_pack::map::Map::from_pack(&b))
                        .and_then(|set| crate::trick_demo::Demo::load(&read).map(|demo| (set, demo)));
                    match loaded {
                        Ok((set, demo)) => {
                            scene.load_demo(set.render);
                            trick_demo = Some(demo);
                            println!("trick guide demo: set loaded in {:.0} ms", started.elapsed().as_secs_f32() * 1000.0);
                        }
                        Err(e) => {
                            println!("trick guide demo off: {e}");
                            demo_pack = Some(false);
                        }
                    }
                }
            } else if demo_wanted.is_none() && trick_demo.is_some() {
                trick_demo = None;
                scene.unload_demo();
                demo_palette = Vec::new();
            }
            let demo_view = match (&demo_wanted, &mut trick_demo) {
                (Some((clip, track)), Some(demo)) => {
                    demo.update(clip, *track, frame_dt, &read);
                    demo.view(&rig_names)
                }
                _ => None,
            };
            if fe_menus.loading_covered() {
                // The retail loading screen (FE 52) is up, or the fader black around it: black under the menus.
                draw_black(&mut apt);
            } else if loading_view {
                apt.draw_screen(LOADING_BACKGROUND);
                if !fe_menus.loading_screen() {
                    draw_spinner(&mut gfx, started.elapsed().as_secs_f32());
                }
            } else if let (Some(v), Some(binding)) = (&demo_view, &mut binding) {
                binding.palette(&v.snapshot, &v.snapshot, 1.0, &mut demo_palette);
                let view_proj = math::projection_far(v.fov.unwrap_or(1.0), 100.0) * math::view(v.camera);
                let eye = v.camera.w_axis.truncate();
                scene.draw_demo(&Frame { view_proj, eye, palette: &demo_palette, props: &[], cars: &[], npcs: &[], peds: &[], hand_props: &[], icons: &[], trails: &[] });
            } else if frozen {
                apt.draw_screen(FROZEN_WORLD);
            } else {
            let mut eye = Vec3::ZERO;
            let mut view_proj = math::projection(1.0) * math::view(Mat4::from_translation(Vec3::new(0.0, 4.0, 14.0)));
            icon_instances.clear();
            trail_instances.clear();
            minimap_view = None;
            if let Some(alpha) = view {
                let (camera, fov) = skin::camera(&a, &b, alpha);
                eye = camera.w_axis.truncate();
                // The teleporter icons of the loaded world, from the player and this camera.
                if let (Some(i), Some(w)) = (teleporter_icons.as_ref(), icon_world) {
                    let world = WORLDS[w];
                    teleporters.icon_anchors(world, &i.key, &mut icon_anchors);
                    let player = a.root.translation.lerp(b.root.translation, alpha);
                    i.instances(world, &icon_anchors, player, &camera, &mut icons);
                    icon_instances.extend(icons.iter().map(|c| vita_render::scene::IconInstance { model: c.model, front: c.front, background: c.background }));
                }
                // The minimap's centre and heading inputs, and its called skaters' icons.
                minimap_view = Some((a.root.translation.lerp(b.root.translation, alpha), -camera.z_axis.truncate()));
                called_positions.clear();
                called_positions.extend(npc_poses.iter().filter(|n| called_serials.contains(&n.serial)).map(|n| n.previous.root.translation.lerp(n.current.root.translation, alpha)));
                if let Some(i) = teleporter_icons.as_ref().filter(|i| i.has_trail()) {
                    called_trails.update(i, &called_serials, &npc_poses, alpha, trail_dt, &rig_names, &camera, &mut scene, &mut trail_instances);
                }
                // A district draws to 250 m; its tiles stream in around the camera.
                let far = if streamer.active() { 260.0 } else { math::FAR };
                view_proj = math::projection_far(fov, far) * math::view(camera);
                if let Some(binding) = &mut binding {
                    binding.palette(&a, &b, alpha, &mut palette);
                }
            }
            let skinned = view.is_some() && binding.is_some();
            streamer.frame(&mut scene, eye, view.is_some());
            prop_instances.clear();
            prop_instances.extend(prop_poses.iter().filter_map(|p| {
                let template = *prop_templates.get(&(p.first_index, p.index_count))?;
                let [x, y, z] = p.basis;
                let [sx, sy, sz] = p.scale;
                let model = Mat4::from_cols((Vec3::from_array(x) * sx).extend(0.0), (Vec3::from_array(y) * sy).extend(0.0), (Vec3::from_array(z) * sz).extend(0.0), Vec3::from_array(p.origin).extend(1.0));
                Some(vita_render::scene::PropInstance { template, model })
            }));
            if !props_logged && !prop_poses.is_empty() {
                props_logged = true;
                let p = &prop_poses[0];
                println!("props: {} poses, {} with a template; first at {:?} basis {:?}", prop_poses.len(), prop_instances.len(), p.origin, p.basis);
            }
            // The cars between their two tick poses (upstream `present_car_pose`: lerp / slerp,
            // the wheel angle forward from the previous one).
            car_instances.clear();
            if let Some(alpha) = view {
                car_instances.extend(car_poses.iter().filter(|c| c.model != u16::MAX).map(|c| {
                    let translation = Vec3::from_array(c.prev.0).lerp(Vec3::from_array(c.curr.0), alpha);
                    let rotation = glam::Quat::from_array(c.prev.1).slerp(glam::Quat::from_array(c.curr.1), alpha);
                    let mut d = c.wheel - c.wheel_prev;
                    if d < 0.0 {
                        d += std::f32::consts::TAU;
                    }
                    let gain = |p: [f32; 4]| [p[0] * c.paint_gain, p[1] * c.paint_gain, p[2] * c.paint_gain, p[3]];
                    vita_render::scene::CarInstance {
                        model: c.model,
                        transform: Mat4::from_rotation_translation(rotation, translation),
                        wheel: c.wheel_prev + d * alpha,
                        chassis: gain(c.chassis),
                        secondary: gain(c.secondary),
                    }
                }));
            }
            // Each NPC skater's palette from its two step poses (the player's binding).
            npc_palettes.truncate(npc_poses.len());
            if let (Some(alpha), Some(binding)) = (view, binding.as_mut()) {
                for (i, n) in npc_poses.iter().enumerate() {
                    if npc_palettes.len() <= i {
                        npc_palettes.push((Vec::new(), 1.0, Vec3::ZERO));
                    }
                    let slot = &mut npc_palettes[i];
                    binding.palette(&n.previous, &n.current, alpha, &mut slot.0);
                    slot.1 = n.alpha;
                    slot.2 = n.previous.root.translation.lerp(n.current.root.translation, alpha);
                }
            } else {
                npc_palettes.clear();
            }
            let npcs: Vec<vita_render::scene::SkinInstance> = npc_palettes.iter().map(|(p, a, c)| vita_render::scene::SkinInstance { palette: p, alpha: *a, center: *c }).collect();
            // Each ped's palette from its model's binding and its two step poses.
            ped_palettes.resize_with(ped_poses.len(), Vec::new);
            let mut peds: Vec<vita_render::scene::PedInstance> = Vec::with_capacity(ped_poses.len());
            if let Some(alpha) = view {
                for (p, slot) in ped_poses.iter().zip(ped_palettes.iter_mut()) {
                    let Some(Some(binding)) = ped_bindings.get_mut(usize::from(p.model)) else { continue };
                    binding.palette(p, alpha, slot);
                    peds.push(vita_render::scene::PedInstance { model: p.model, palette: slot, center: p.previous.0.lerp(p.current.0, alpha), tint_a: p.tint_a, tint_b: p.tint_b, alpha: p.alpha, lod: p.lod });
                }
            }
            // The hand props: held ones at their ped's blended hand bone (the ped's opacity), released
            // ones between their two body frames.
            hand_frames.clear();
            if let Some(alpha) = view {
                for p in &ped_poses {
                    let (Some((model, local)), Some(bone)) = (p.hand, hand_bone) else { continue };
                    let Some(Some(binding)) = ped_bindings.get(usize::from(p.model)) else { continue };
                    if let Some(m) = binding.hand_world(p, alpha, bone, local) {
                        hand_frames.push((model, m, p.alpha));
                    }
                }
                for h in &hand_poses {
                    hand_frames.push((h.model, h.previous * (1.0 - alpha) + h.current * alpha, 1.0));
                }
            }
            let hand_instances: Vec<vita_render::scene::PedInstance> = hand_frames
                .iter()
                .map(|(model, m, a)| vita_render::scene::PedInstance { model: *model, palette: std::slice::from_ref(m), center: m.w_axis.truncate() - glam::Vec3::Y, tint_a: [0.0; 4], tint_b: [0.0; 4], alpha: *a, lod: 0 })
                .collect();
            let grade = if settings.zombie {
                *zombie_grade.get_or_insert_with(|| crate::colour_grade::zombie(std::path::Path::new(crate::packs::ROOT)))
            } else {
                None
            };
            scene.set_grade(grade.unwrap_or(vita_render::scene::Grade::IDENTITY));
            scene.draw(&Frame { view_proj, eye, palette: if skinned { &palette } else { &[] }, props: &prop_instances, cars: &car_instances, npcs: &npcs, peds: &peds, hand_props: &hand_instances, icons: &icon_instances, trails: &trail_instances });
            if freeze && paused_frames >= FREEZE_AFTER_FRAMES {
                let started = Instant::now();
                apt.add_texture(FROZEN_WORLD, math::WIDTH as u32, math::HEIGHT as u32, &vita_render::read_rgba());
                println!("pause: world captured in {:.1} ms", started.elapsed().as_secs_f32() * 1000.0);
            }
            }
            if photo_request {
                // The picture without the editor's panels.
                let text = match crate::session_replay::save_photo(vita_render::screenshot_bmp()) {
                    Ok(name) => format!("Photo saved: {name}"),
                    Err(e) => format!("Photo failed: {e}"),
                };
                photo_note = Some((text, Instant::now()));
            }
            stats.draws = scene.last_draws;
            if let Some(m) = &mut marker {
                m.advance();
            }
            // Open over the pause menu, or its outro over the game; the controller glyphs its prompts leave to us.
            let fe_glyphs = fe_menus.draw(&mut apt);
            let icons: Vec<_> = fe_glyphs
                .iter()
                .filter_map(|(name, [x, y, w, h])| crate::button_icons::Icon::of(name).map(|icon| crate::button_icons::draw(icon, *x, *y, w.min(*h))))
                .collect();
            apt.draw(&icons);
            replay_glyphs = Vec::new();
            // The menus' plates (L, R, the touch zones, the D-pad...) are drawn with the replay screens' (next overlay).
            replay_glyphs.extend(fe_glyphs);
            if screen == Screen::Replay && retail_replay {
                // The retail screen, its help and the Finished popup drew with the menus (their glyphs came with them).
                let icons: Vec<_> = replay_glyphs
                    .iter()
                    .filter_map(|(name, [x, y, w, h])| crate::button_icons::Icon::of(name).map(|icon| crate::button_icons::draw(icon, *x, *y, w.min(*h))))
                    .collect();
                apt.draw(&icons);
            } else if screen != Screen::Replay {
                help_open = false;
            }
            if screen != Screen::Replay {
                // The retail replay screen closes with the replay; the next one starts with the first help page.
                if retail_replay {
                    fe_menus.replay(None, frame_dt, &[0.0; 24]);
                }
                retail_replay = false;
                help_turns = 0;
                finish_popup = false;
            }
            if screen == Screen::Playing && !fe_menus.loading_screen() {
                if let Some(m) = &mut marker {
                    m.draw_dissolve(&marker_view);
                }
                retail.draw(&mut apt);
                if let Some(m) = &mut meter {
                    if m.frame(&if settings.manual_meter { manual_view } else { Default::default() }) {
                        m.draw(&mut apt);
                    } else {
                        println!("manual meter off: script error");
                        meter = None;
                    }
                }
                if let (Some(m), Some((player, forward))) = (&mut minimap, minimap_view) {
                    if m.frame(frame_dt, settings.minimap, player, forward, &called_positions) {
                        m.draw(&mut apt);
                    } else {
                        println!("minimap off: script error");
                        minimap = None;
                    }
                }
                if settings.trick_analyzer {
                    if let Some(a) = &mut analyzer {
                        if a.frame(&score_view) {
                            a.draw(&mut apt);
                        } else {
                            println!("trick analyzer off: script error");
                            analyzer = None;
                        }
                    }
                }
                if let Some(m) = &marker {
                    m.draw_panel(&mut apt, &marker_view);
                }
            }
            let damaged = crate::packs::DAMAGED.lock().unwrap().clone();
            if let Some(problem) = damaged.or_else(|| pack_error.clone()).or_else(|| binding_error.clone()).or_else(|| retail.failure().map(|e| format!("retail HUD off: {e}")))
                .or_else(|| fe_menus.failure().map(|e| format!("retail menus off: {e}"))) {
                gfx.text(12.0, 530.0, 1.0, RED, &lab::truncate(&problem, 110));
            }
            gfx.draw();
            stats.cpu_us_sum += frame_start.elapsed().as_micros() as u64;
            vita_render::swap(fe_menus.keyboard_running());
        }
        stats.frame(&info);
    }
    settings.save();
    stats.log_heap(0);
    println!("quit, exiting");
    // Let PrincessLog send the last line before the process goes away.
    std::thread::sleep(Duration::from_millis(300));
    unsafe { vitasdk_sys::sceKernelExitProcess(0) };
    unreachable!()
}

fn volume(settings: &Settings) -> f32 {
    f32::from(settings.volume) / 10.0
}

/// One frame's buttons on the current screen. Returns whether the settings changed.
fn handle_input(
    screen: &mut Screen,
    menus: &mut Menus,
    settings: &mut Settings,
    s: &mut Shared,
    held: u32,
    pressed: u32,
    quit: &mut bool,
    boot: bool,
) -> bool {
    // Select + Triangle on any menu: the retail crossbar or the plain menus.
    if matches!(*screen, Screen::Main | Screen::Pause | Screen::Settings { .. }) && held & sce::SELECT != 0 && pressed & sce::TRIANGLE != 0 {
        settings.retail_menus = !settings.retail_menus;
        return true;
    }
    // Select + Square on the pause menu: replay the last minute.
    if *screen == Screen::Pause && held & sce::SELECT != 0 && pressed & sce::SQUARE != 0 && crate::session_replay::RECORDER.lock().unwrap().len() > 1 {
        *screen = Screen::Replay;
        return false;
    }
    match *screen {
        Screen::Loading | Screen::Replay => false,
        Screen::Playing => {
            // Select alone (a tap, no other button during it): the replay, as in the retail game.
            use std::sync::atomic::{AtomicU8, Ordering};
            // 0 up, 1 down and clean, 2 down and used in a chord.
            static SELECT: AtomicU8 = AtomicU8::new(0);
            let state = SELECT.load(Ordering::Relaxed);
            if held & sce::SELECT != 0 {
                let used = pressed & !sce::SELECT != 0 || (state == 0 && held & !sce::SELECT != 0);
                SELECT.store(if used || state == 2 { 2 } else { 1 }, Ordering::Relaxed);
            } else {
                SELECT.store(0, Ordering::Relaxed);
                if state == 1 && !AUTOPLAY && crate::session_replay::RECORDER.lock().unwrap().len() > 1 {
                    *screen = Screen::Replay;
                    return false;
                }
            }
            if pressed & sce::START != 0 {
                if held & sce::SELECT != 0 {
                    settings.debug_hud = !settings.debug_hud;
                    return true;
                }
                if !AUTOPLAY {
                    *screen = Screen::Pause;
                    crate::session_replay::refresh_list();
                    menus.cursors[1] = 0;
                    menus.world = s.world_loaded;
                }
            }
            false
        }
        Screen::Calibration { .. } => {
            if pressed & sce::START != 0 {
                *screen = screen.parent();
            } else if held & sce::SELECT != 0 && pressed & sce::CIRCLE != 0 {
                s.reset = true;
            }
            false
        }
        Screen::Controls { .. } => {
            if pressed & (sce::START | sce::CIRCLE | sce::CROSS) != 0 {
                *screen = screen.parent();
            }
            false
        }
        Screen::Main | Screen::Pause | Screen::Settings { .. } => {
            let (slot, mut m, actions) = build_menu(*screen, s, settings, menus.world);
            m.cursor = menus.cursors[slot].min(m.rows.len() - 1);
            let input = m.input(pressed);
            menus.cursors[slot] = m.cursor;
            let (action, step) = match input {
                menu::Input::None => return false,
                menu::Input::Back => {
                    if *screen != Screen::Main {
                        *screen = screen.parent();
                    }
                    return false;
                }
                menu::Input::Choose(i) => (actions[i], 1),
                menu::Input::Change(i, step) => (actions[i], step),
            };
            let chosen = matches!(input, menu::Input::Choose(_));
            let paused = *screen != Screen::Main;
            let toggle = |b: &mut bool| *b = !*b;
            match action {
                Action::Skate | Action::Park if chosen && boot && !paused => {}
                Action::Skate | Action::Park if chosen => {
                    // (Re)load when another park is selected or the last load failed.
                    *screen = Screen::Playing;
                    if menus.world != s.world_loaded || s.error.is_some() && !s.loading {
                        s.world = menus.world;
                        s.reload = true;
                        settings.world = menus.world as u8;
                        return true;
                    }
                }
                Action::Park => {
                    menus.world = (menus.world as i32 + step).rem_euclid(WORLDS.len() as i32) as usize;
                }
                Action::Replay if chosen => {
                    menus.replay_request = Some(menus.replay);
                }
                Action::Replay => {
                    let n = crate::session_replay::choices().len() as i32;
                    menus.replay = (menus.replay as i32 + step).rem_euclid(n) as usize;
                    REPLAY_CHOICE.store(menus.replay, std::sync::atomic::Ordering::Relaxed);
                }
                Action::Resume => {
                    menus.world = s.world_loaded;
                    *screen = Screen::Playing;
                }
                Action::Respawn => {
                    s.respawn = true;
                    *screen = Screen::Playing;
                }
                Action::Difficulty => {
                    settings.difficulty = settings.difficulty.next();
                    if step < 0 {
                        settings.difficulty = settings.difficulty.next();
                    }
                    return true;
                }
                Action::Physics => {
                    toggle(&mut settings.fast_physics);
                    return true;
                }
                Action::AiSkaters => {
                    toggle(&mut settings.ai_skaters);
                    return true;
                }
                Action::Pedestrians => {
                    let n = crate::settings::PEDESTRIANS.len() as u8;
                    settings.pedestrians = if step < 0 { (settings.pedestrians + n - 1) % n } else { (settings.pedestrians + 1) % n };
                    return true;
                }
                Action::Traffic => {
                    let n = crate::settings::TRAFFIC.len() as u8;
                    settings.traffic = if step < 0 { (settings.traffic + n - 1) % n } else { (settings.traffic + 1) % n };
                    return true;
                }
                Action::Controls => *screen = Screen::Controls { paused },
                Action::Calibration => *screen = Screen::Calibration { paused },
                Action::Settings => {
                    menus.cursors[2] = 0;
                    *screen = Screen::Settings { paused };
                }
                Action::MainMenu => {
                    menus.cursors[0] = 0;
                    *screen = Screen::Main;
                }
                Action::Quit => *quit = true,
                Action::RearTouch => {
                    toggle(&mut settings.controls.rear_touch_bumpers);
                    return true;
                }
                Action::DpadBumpers => {
                    toggle(&mut settings.controls.dpad_bumpers);
                    return true;
                }
                Action::FrontTouch => {
                    toggle(&mut settings.controls.front_touch_thumbs);
                    return true;
                }
                Action::Volume => {
                    settings.volume = if chosen {
                        (settings.volume + 1) % 11
                    } else {
                        (i32::from(settings.volume) + step).clamp(0, 10) as u8
                    };
                    return true;
                }
                Action::DebugHud => {
                    toggle(&mut settings.debug_hud);
                    return true;
                }
                Action::Audio => {
                    toggle(&mut settings.retail_audio);
                    return true;
                }
                Action::ManualMeter => {
                    toggle(&mut settings.manual_meter);
                    return true;
                }
                Action::TrickAnalyzer => {
                    toggle(&mut settings.trick_analyzer);
                    return true;
                }
                Action::MenuStyle => {
                    toggle(&mut settings.retail_menus);
                    return true;
                }
                Action::Back => *screen = screen.parent(),
                Action::Skate => {}
            }
            false
        }
    }
}

/// Starts (or resumes) play; loads another park when one is selected or the last load
/// failed. Returns whether the settings changed (the selected park is saved).
/// The worlds of the retail pause menu's lists (`vita_menus`): the parks, the districts, then Skate School and the
/// course (the crossbar's column holds six rows).
fn menu_worlds() -> crate::vita_menus::Worlds {
    let at = |name: &str| WORLDS.iter().position(|w| *w == name).unwrap_or(0);
    let list = |names: &[(&'static str, &'static str)]| names.iter().map(|(w, label)| (at(w), *label)).collect();
    crate::vita_menus::Worlds {
        parks: list(&[
            ("StartPark", "Start Park"),
            ("MegaPark", "Mega Park"),
            ("BlackBoxPark", "Black Box"),
            ("IndustrialSkatePark", "Industrial Park"),
            ("DownTownSkatePark", "Downtown Park"),
            ("MaloofMoneyCup", "Maloof Cup"),
        ]),
        districts: list(&[("Industrial", "Industrial"), ("University", "University"), ("DownTown", "Downtown")]),
        training: list(&[("SkateSchool", "Skate School"), ("Course", "Course")]),
    }
}

fn start_playing(screen: &mut Screen, menus: &Menus, settings: &mut Settings, s: &mut Shared) -> bool {
    *screen = Screen::Playing;
    if menus.world != s.world_loaded || s.error.is_some() && !s.loading {
        s.world = menus.world;
        s.reload = true;
        settings.world = menus.world as u8;
        return true;
    }
    false
}

/// A menu screen's rows and their actions, and the index of its remembered cursor.
fn build_menu(screen: Screen, s: &Shared, settings: &Settings, world: usize) -> (usize, Menu, Vec<Action>) {
    let on = |b: bool| if b { "on" } else { "off" };
    let park = || {
        let name = WORLDS[world];
        if world == s.world_loaded && s.error.is_none() { name.to_owned() } else { format!("{name} *") }
    };
    let replay_choice = || {
        let choices = crate::session_replay::choices();
        choices[REPLAY_CHOICE.load(std::sync::atomic::Ordering::Relaxed).min(choices.len() - 1)].clone()
    };
    let (slot, title, rows) = match screen {
        Screen::Main => (0, "Skate Vita", vec![
            (Row::action("Skate"), Action::Skate),
            (Row::choice("Park", park()), Action::Park),
            (Row::choice("Difficulty", settings.difficulty.label()), Action::Difficulty),
            (Row::choice("Physics", if settings.fast_physics { "Fast" } else { "Retail" }), Action::Physics),
            (Row::action("Controls"), Action::Controls),
            (Row::action("Stick calibration"), Action::Calibration),
            (Row::action("Settings"), Action::Settings),
            (Row::action("Quit"), Action::Quit),
        ]),
        Screen::Pause => (1, "Paused", vec![
            (Row::action("Resume"), Action::Resume),
            (Row::action("Respawn"), Action::Respawn),
            (Row::choice("Replay editor", replay_choice()), Action::Replay),
            (Row::choice("Park", park()), Action::Park),
            (Row::choice("Difficulty", settings.difficulty.label()), Action::Difficulty),
            (Row::choice("Physics", if settings.fast_physics { "Fast" } else { "Retail" }), Action::Physics),
            (Row::action("Controls"), Action::Controls),
            (Row::action("Settings"), Action::Settings),
            (Row::action("Main menu"), Action::MainMenu),
            (Row::action("Quit"), Action::Quit),
        ]),
        _ => {
            let c = &settings.controls;
            (2, "Settings", vec![
                (Row::choice("Rear touch = LB/RB", on(c.rear_touch_bumpers)), Action::RearTouch),
                (Row::choice("D-pad L/R = LB/RB", on(c.dpad_bumpers)), Action::DpadBumpers),
                (Row::choice("Front touch = L3/R3", on(c.front_touch_thumbs)), Action::FrontTouch),
                (Row::choice("Volume", settings.volume.to_string()), Action::Volume),
                (Row::choice("Traffic", settings.traffic_label()), Action::Traffic),
                (Row::choice("AI skaters", on(settings.ai_skaters)), Action::AiSkaters),
                (Row::choice("Pedestrians", settings.pedestrians_label()), Action::Pedestrians),
                (Row::choice("Trick analyzer", on(settings.trick_analyzer)), Action::TrickAnalyzer),
                (Row::choice("Manual meter", on(settings.manual_meter)), Action::ManualMeter),
                (Row::choice("Audio", if settings.retail_audio { "Retail" } else { "Synth" }), Action::Audio),
                (Row::choice("Debug overlay", on(settings.debug_hud)), Action::DebugHud),
                (Row::choice("Menu style", if settings.retail_menus { "Retail" } else { "Plain" }), Action::MenuStyle),
                (Row::action("Back"), Action::Back),
            ])
        }
    };
    let mut menu = Menu::new(title);
    let (rows, actions) = rows.into_iter().unzip();
    menu.rows = rows;
    (slot, menu, actions)
}

struct Info {
    world: &'static str,
    state: String,
    speed: f32,
    tick_ms: f32,
    tick_max_ms: f32,
    dropped: u64,
    animation_bytes: usize,
}

/// Everything drawn over the 3D view.
struct Overlay<'a> {
    screen: Screen,
    menus: &'a Menus,
    settings: &'a Settings,
    names: Option<&'a Names>,
    line_peak: f32,
    info: &'a Info,
    stats: &'a Stats,
    /// The original HUD movie is drawing: the basic HUD stays off.
    retail_hud: bool,
    /// The R2 front end is open over the pause menu.
    fe_menu: bool,
    /// Triangle asked for the R2 front end, still loading.
    fe_waiting: bool,
    /// The loading screen shows (startup, a world loading).
    loading_view: bool,
    /// The UI loaders are still running: starting play waits.
    boot: bool,
    /// The HUD movie of the new session is being built: the simulation is held.
    hud_wait: bool,
    /// The replay's status line.
    replay: &'a str,
}

const SHADE: Color = [0.0, 0.0, 0.0, 0.7];

/// Text with a drop shadow; returns the x after it.
fn shadow(g: &mut Gfx, x: f32, y: f32, scale: f32, c: Color, text: &str) -> f32 {
    g.text(x + scale, y + scale, scale, SHADE, text);
    g.text(x, y, scale, c, text)
}

fn width(text: &str, scale: f32) -> f32 {
    text.chars().count() as f32 * 8.0 * scale
}

impl Overlay<'_> {
    fn draw(&self, g: &mut Gfx, s: &Shared) {
        // The loading screen is the picture and its circle only.
        if self.loading_view {
            return;
        }
        if self.fe_waiting {
            shadow(g, 40.0, 496.0, 1.0, WHITE, "Loading the retail pause menu... (it opens when ready)");
        }
        match self.screen {
            Screen::Loading => {}
            Screen::Playing if self.retail_hud => {}
            Screen::Playing => self.hud(g, &s.score),
            Screen::Controls { .. } => controls_page(g),
            Screen::Main | Screen::Pause if self.fe_menu => {}
            Screen::Main | Screen::Pause | Screen::Settings { .. } => self.menu(g, s),
            Screen::Replay => {
                shadow(g, 12.0, 516.0, 1.0, WHITE, self.replay);
            }
            Screen::Calibration { .. } => {}
        }
        if self.settings.debug_hud {
            // Large blocks (alloc.rs): `vita::heap()` walks the whole heap under the malloc lock.
            let (used, arena) = crate::alloc::large_blocks();
            let (i, st) = (self.info, self.stats);
            g.text(8.0, 4.0, 1.0, GREY, &format!(
                "{:.0} fps  cpu {:.1} ms  sim {:.1} ms (max {:.1})  dropped {}  holds {}  heap {:.1} MB (peak {:.1})",
                st.fps, st.cpu_ms, i.tick_ms, i.tick_max_ms, i.dropped, st.holds_total,
                used as f32 / 1048576.0, arena as f32 / 1048576.0
            ));
            g.text(8.0, 14.0, 1.0, GREY, &format!(
                "{}  {}  {:.1} m/s  anim cache {:.1} MB",
                i.world, i.state, i.speed, i.animation_bytes as f32 / 1048576.0
            ));
        }
        let status = if s.recovering {
            Some(format!("Recovering, reloading {}...", WORLDS[s.world]))
        } else if s.loading {
            Some(format!("Loading {}...", WORLDS[s.world]))
        } else if self.hud_wait {
            Some("Loading the HUD...".to_owned())
        } else if self.boot && self.screen == Screen::Main {
            Some("Loading the menus and the HUD...".to_owned())
        } else {
            s.error.as_ref().map(|e| lab::truncate(e, 52))
        };
        if let Some(status) = status {
            let w = width(&status, 2.0);
            let x = (480.0 - w / 2.0).max(8.0);
            g.rect(x - 16.0, 448.0, w + 32.0, 40.0, SHADE);
            shadow(g, x, 460.0, 2.0, if s.error.is_some() && !s.loading { RED } else { YELLOW }, &status);
        }
    }

    /// Trick and sequence (top left), line and its timer (top right), total (bottom).
    fn hud(&self, g: &mut Gfx, score: &live::Score) {
        let top = if self.settings.debug_hud { 34.0 } else { 20.0 };
        if let Some(trick) = self.names.and_then(|n| hud::trick_line(n, score)) {
            shadow(g, 24.0, top, 2.0, YELLOW, &lab::truncate(&trick, 34));
        }
        if score.sequence > 0.0 {
            let x = shadow(g, 24.0, top + 22.0, 3.0, WHITE, &hud::points(score.sequence));
            if score.multiplier > 1.0 {
                shadow(g, x + 10.0, top + 30.0, 2.0, CYAN, &format!("x{:.1}", score.multiplier));
            }
            let tag = if score.clean { Some(("CLEAN", GREEN)) } else if score.sketchy { Some(("SKETCHY", GREY)) } else { None };
            if let Some((text, colour)) = tag {
                shadow(g, 24.0, top + 50.0, 1.0, colour, text);
            }
        }
        if score.line > 0.0 {
            let text = format!("LINE {}", hud::points(score.line));
            shadow(g, 936.0 - width(&text, 2.0), top, 2.0, WHITE, &text);
            let left = if self.line_peak > 0.0 { (score.line_time / self.line_peak).clamp(0.0, 1.0) } else { 0.0 };
            g.rect(736.0, top + 22.0, 200.0, 4.0, SHADE);
            g.rect(736.0, top + 22.0, 200.0 * left, 4.0, YELLOW);
        }
        shadow(g, 24.0, 506.0, 2.0, WHITE, &format!("SCORE {}", hud::points(score.total)));
    }

    fn menu(&self, g: &mut Gfx, s: &Shared) {
        let (slot, m, _) = build_menu(self.screen, s, self.settings, self.menus.world);
        let cursor = self.menus.cursors[slot].min(m.rows.len() - 1);
        g.rect(0.0, 0.0, 440.0, 544.0, SHADE);
        shadow(g, 40.0, 40.0, 3.0, WHITE, &m.title);
        // 40 px apart, closer when the rows would reach the hint line (the last row ends above 512).
        let step = (392.0 / (m.rows.len().max(2) - 1) as f32).min(40.0);
        for (i, row) in m.rows.iter().enumerate() {
            let y = 104.0 + i as f32 * step;
            let colour = if i == cursor { YELLOW } else { WHITE };
            if i == cursor {
                g.rect(28.0, y - 8.0, 396.0, 32.0, [1.0, 1.0, 1.0, 0.1]);
            }
            shadow(g, 40.0, y, 2.0, colour, row.label);
            if let Some(v) = &row.value {
                let value = format!("< {v} >");
                let scale = if width(&value, 2.0) > 200.0 { 1.0 } else { 2.0 };
                shadow(g, 412.0 - width(&value, scale), y + 8.0 - 4.0 * scale, scale, colour, &value);
            }
        }
        let hint = if self.screen == Screen::Main {
            "Up/Down move  Left/Right change  Cross choose"
        } else {
            "Up/Down move  Left/Right change  Cross choose  Circle back"
        };
        shadow(g, 40.0, 516.0, 1.0, GREY, hint);
        if m.rows.iter().any(|r| r.value.as_deref().is_some_and(|v| v.ends_with(" *"))) {
            shadow(g, 40.0, 500.0, 1.0, GREY, "* loads when you skate");
        }
    }
}

/// The Vita buttons and what they send (the game's own Xbox 360 layout).
fn controls_page(g: &mut Gfx) {
    g.rect(0.0, 0.0, 960.0, 544.0, [0.0, 0.0, 0.0, 0.8]);
    shadow(g, 48.0, 36.0, 3.0, WHITE, "Controls");
    shadow(g, 48.0, 70.0, 1.0, GREY, "Skate 3's Xbox 360 controls, on the Vita:");
    let lines = [
        ("Left stick", "Left stick", "steer, lean, spin in the air"),
        ("Right stick", "Right stick", "flick-it tricks, manual, grinds"),
        ("Cross", "A", "push; recover after a bail"),
        ("Square", "X", "mongo push; jump off the board"),
        ("Circle", "B", "brake, powerslide; dismount"),
        ("Triangle", "Y", "get off / on the board"),
        ("L / R", "LT / RT", "left / right grab"),
        ("Rear touch left / right", "LB / RB", "modifier / grab the world (setting)"),
        ("D-pad left / right", "LB / RB", "same, instead of the rear touch (setting)"),
        ("Front touch left / right", "L3 / R3", "bail chord (setting)"),
        ("Start", "", "pause menu"),
        ("Select + Start", "", "debug overlay"),
    ];
    for (i, (vita, xbox, action)) in lines.iter().enumerate() {
        let y = 100.0 + i as f32 * 30.0;
        shadow(g, 48.0, y, 2.0, YELLOW, vita);
        shadow(g, 476.0, y, 2.0, WHITE, xbox);
        shadow(g, 640.0, y + 4.0, 1.0, GREY, action);
    }
    shadow(g, 48.0, 516.0, 1.0, GREY, "Circle: back");
}

/// The ring entry to show at `now` and its blend factor: the newest pair whose tick
/// ran at or before `now - 1.5 periods`. `hold` when even the newest pair is overdue.
fn pick(ring: &std::collections::VecDeque<lab::Published>, now: Instant) -> Option<(usize, f32, bool)> {
    let newest = ring.back()?;
    let v = now.checked_sub(newest.period + newest.period / 2)?;
    Some(match ring.iter().rposition(|e| e.due <= v) {
        Some(i) => {
            let e = &ring[i];
            let alpha = (v - e.due).as_secs_f32() / e.period.as_secs_f32();
            (i, alpha.min(1.0), alpha > 1.0 && i + 1 == ring.len())
        }
        None => (0, 0.0, false),
    })
}

fn empty_snapshot() -> Snapshot {
    Snapshot { root: live::Transform::IDENTITY, bones: Vec::new(), camera: live::Transform::IDENTITY, fov: 1.0 }
}

/// Frame statistics, logged once per second; the heap every 10 s (the in-use
/// high-water mark must not grow over a session).
#[derive(Default)]
struct Stats {
    second: Option<Instant>,
    seconds: u64,
    frames: u32,
    cpu_us_sum: u64,
    holds: u32,
    holds_total: u64,
    fps: f32,
    cpu_ms: f32,
    draws: (u32, u32),
    heap_peak: usize,
}
impl Stats {
    fn frame(&mut self, info: &Info) {
        self.frames += 1;
        let start = *self.second.get_or_insert_with(Instant::now);
        let elapsed = start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            self.fps = self.frames as f32 / elapsed.as_secs_f32();
            self.cpu_ms = self.cpu_us_sum as f32 / 1000.0 / self.frames.max(1) as f32;
            println!(
                "RENDER fps={:.1} cpu_ms={:.2} holds={} draws={} culled={} sim_ms={:.1} sim_max_ms={:.1} dropped={} world={} state={}",
                self.fps, self.cpu_ms, self.holds, self.draws.0, self.draws.1, info.tick_ms, info.tick_max_ms, info.dropped, info.world, info.state
            );
            self.seconds += 1;
            if self.seconds % 10 == 0 {
                self.log_heap(info.animation_bytes);
            }
            self.frames = 0;
            self.cpu_us_sum = 0;
            self.holds = 0;
            self.second = Some(Instant::now());
        }
    }

    fn log_heap(&mut self, animation_bytes: usize) {
        // Large blocks only (see alloc.rs): `vita::heap()` (mallinfo) walks the heap under the malloc
        // lock, 265 ms of simulation stall every time on a 150 MB heap.
        let (used, peak) = crate::alloc::large_blocks();
        self.heap_peak = peak;
        // Diagnostic (retail-audio memory budget): newlib's own count (walks the heap: a hitch)
        // and vitaGL's pools.
        #[cfg(feature = "mem-log")]
        {
            let (in_use, arena, free, chunks) = vita_platform::vita::heap_detail();
            let pool = |k| unsafe { (vita_render::vgl::vglMemFree(k) >> 20, vita_render::vgl::vglMemTotal(k) >> 20) };
            let (vram, ram, slow, budget) = (pool(0), pool(1), pool(2), pool(3));
            println!(
                "MEM t={}s newlib in_use={} MiB arena={} MiB free={} MiB in {} chunks; vitaGL free/total MiB: vram {}/{} ram {}/{} slow {}/{} budget {}/{}",
                self.seconds, in_use >> 20, arena >> 20, free >> 20, chunks, vram.0, vram.1, ram.0, ram.1, slow.0, slow.1, budget.0, budget.1
            );
            crate::alloc::stage(&format!("t={}s", self.seconds));
        }
        println!(
            "HEAP t={}s in_use={} peak={} small={} returned={} arena=0 anim={}",
            self.seconds,
            used,
            self.heap_peak,
            crate::alloc::small_blocks(),
            crate::alloc::returned_at_exit(),
            animation_bytes
        );
    }
}

/// The controller glyphs the retail screens leave to the host that are not textured icons
/// (`button_icons`): a bordered plate with the button's name for L, R, Start and the touch zones,
/// a D-pad cross with the active arms lit, and the letter on a stick. Everything stays inside the
/// slot, as the retail glyphs do.
fn draw_glyphs(g: &mut Gfx, glyphs: &[(String, [f32; 4])]) {
    const INK: Color = [1.0, 1.0, 1.0, 1.0];
    const RIM: Color = [0.86, 0.88, 0.92, 1.0];
    const PLATE: Color = [0.07, 0.09, 0.13, 0.92];
    for (name, [x, y, w, h]) in glyphs {
        // (x, y) is the slot's centre in the 1280 x 720 stage.
        let (sw, sh) = (w * 0.75, h * 0.75);
        let (cx, cy) = (x * 0.75, y * 0.75 + 1.0);
        let (sx, sy) = (cx - sw / 2.0, cy - sh / 2.0);
        let r = sw.min(sh) / 2.0;
        // Text that fits the slot: 8 px characters, smaller for long names.
        let fitted = |g: &mut Gfx, text: &str, room: f32| {
            let n = text.chars().count() as f32;
            let scale = (room / (8.0 * n)).min(1.0);
            g.text(cx - 4.0 * n * scale, cy - 4.0 * scale, scale, INK, text);
        };
        let plate = |g: &mut Gfx, text: &str| {
            let (pw, ph) = (sw - 2.0, 14.0);
            g.rect(cx - pw / 2.0, cy - ph / 2.0, pw, ph, RIM);
            g.rect(cx - pw / 2.0 + 1.0, cy - ph / 2.0 + 1.0, pw - 2.0, ph - 2.0, PLATE);
            fitted(g, text, pw - 4.0);
        };
        match name.as_str() {
            "Start" => plate(g, "START"),
            "Back" => plate(g, "SELECT"),
            // LT and RT are the Vita's L and R; LB and RB the rear touch halves; L3 the front touch's left half.
            "L_Trigger_hud" => plate(g, "L"),
            "R_Trigger_hud" => plate(g, "R"),
            "L_Bumper_hud" => plate(g, "rL"),
            "R_Bumper_hud" => plate(g, "rR"),
            "L3_Click_hud" => plate(g, "fL"),
            "R3_Click_hud" => plate(g, "fR"),
            "Stick_Left_hud" => fitted(g, "L", sw),
            "Stick_Right_hud" => fitted(g, "R", sw),
            "DPad_Left_hud" | "DPad_Right_hud" | "DPad_Up_hud" | "DPad_Down_hud" | "DPad_UpDown_hud" | "DPad_Arrows_hud" => {
                // A cross of four arms on a dark plate, the pressed ones bright.
                g.rect(sx + 2.0, sy + 2.0, sw - 4.0, sh - 4.0, PLATE);
                let lit = |dx: f32, dy: f32| match name.as_str() {
                    "DPad_Left_hud" => dx < 0.0,
                    "DPad_Right_hud" => dx > 0.0,
                    "DPad_Up_hud" => dy < 0.0,
                    "DPad_Down_hud" => dy > 0.0,
                    "DPad_UpDown_hud" => dy != 0.0,
                    _ => dx != 0.0,
                };
                let dim: Color = [0.42, 0.47, 0.56, 1.0];
                g.rect(cx - 2.5, cy - 2.5, 5.0, 5.0, dim);
                for (dx, dy) in [(-1.0f32, 0.0f32), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                    g.rect(cx + dx * r * 0.5 - 2.5, cy + dy * r * 0.5 - 2.5, 5.0, 5.0, if lit(dx, dy) { INK } else { dim });
                }
            }
            _ => {}
        }
    }
}

/// The replay editor's timeline (the retail game draws it natively): in the instant replay a faint
/// yellow strip; in the timeline editor each clip in the colour of its camera, bright under the
/// playhead and dimmer elsewhere, dark outside the In and Out points; the clip markers and the
/// camera keys. The strip is 228 to 1046 in the 1280 x 720 stage, 53 to 83 tall; `band` is
/// (In, Out, editing the timeline).
fn draw_timeline(g: &mut Gfx, band: Option<(f32, f32, bool)>, zones: &[(f32, f32, crate::replay_camera::Kind, bool)], markers: &[f32], keys: &[f32]) {
    use crate::replay_camera::Kind;
    let Some((start, end, editing)) = band else { return };
    let (left, width, top, height) = (228.0 * 0.75, 818.0 * 0.75, 53.0 * 0.75 + 2.0, 30.0 * 0.75);
    if editing {
        g.rect(left, top, width, height, [0.08, 0.14, 0.2, 0.35]);
        for &(a, b, kind, here) in zones {
            let (a, b) = (a.max(start), b.min(end));
            if b <= a {
                continue;
            }
            // Game camera yellow, Follow blue, Tripod grey, Free violet.
            let colour: [f32; 3] = match kind {
                Kind::Game => [0.82, 0.8, 0.14],
                Kind::Follow => [0.36, 0.68, 0.84],
                Kind::Tripod => [0.62, 0.65, 0.68],
                Kind::Free => [0.62, 0.52, 0.86],
            };
            let (shade, alpha) = if here { (1.0, 0.8) } else { (0.5, 0.65) };
            g.rect(left + a * width, top, (b - a) * width, height, [colour[0] * shade, colour[1] * shade, colour[2] * shade, alpha]);
        }
    } else {
        g.rect(left, top, width, height, [0.8, 0.78, 0.18, 0.32]);
    }
    for f in markers {
        g.rect(left + f * width - 1.5, top - 3.0, 3.0, height + 6.0, [1.0, 0.55, 0.1, 1.0]);
    }
    // Camera key markers of the advanced cameras: small dark keys with a light rim.
    for f in keys {
        let x = left + f * width;
        g.rect(x - 4.0, top + height - 9.0, 8.0, 13.0, [0.85, 0.88, 0.92, 1.0]);
        g.rect(x - 3.0, top + height - 8.0, 6.0, 11.0, [0.18, 0.2, 0.24, 1.0]);
    }
}

/// The Finished menu drawn plainly while the retail popup is not loaded: Upload Video (not
/// available), Save Replay, Exit.
fn draw_finish_menu(g: &mut Gfx, item: usize) {
    let (x, y, w, h) = (330.0, 170.0, 300.0, 170.0);
    g.rect(x, y, w, h, [0.05, 0.07, 0.12, 0.92]);
    g.rect(x, y, w, 24.0, [0.2, 0.28, 0.4, 1.0]);
    g.text(x + 12.0, y + 8.0, 1.0, [0.96, 0.96, 0.98, 1.0], "Finished");
    for (i, label) in ["Upload Video", "Save Replay", "Exit"].iter().enumerate() {
        let top = y + 38.0 + i as f32 * 28.0;
        let colour: Color = if i == 0 { [0.45, 0.47, 0.52, 1.0] } else { [0.96, 0.96, 0.98, 1.0] };
        if i == item {
            g.rect(x + 8.0, top - 5.0, w - 16.0, 22.0, [0.25, 0.4, 0.65, 0.9]);
        }
        g.text(x + 20.0, top, 1.0, colour, label);
    }
    let status = crate::session_replay::SAVE_STATUS.lock().unwrap().clone();
    g.text(x + 12.0, y + h - 36.0, 1.0, [0.7, 0.85, 1.0, 1.0], &status);
    g.text(x + 12.0, y + h - 16.0, 1.0, [0.7, 0.72, 0.8, 1.0], "A select   B cancel");
}
