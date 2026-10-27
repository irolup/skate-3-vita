//! R2 (device test): the retail front end on upstream's APT runtime (`skate_ui::fe::menu_stage`): the pause menu,
//! Game Settings, the popups and Video Calibration. It loads on a thread after the other UI loaders (the movie set
//! and its textures from ui.pak), uploads the textures a few per frame and runs on the main thread while open.
//! It is the pause menu (R2c): it opens whenever the game enters the pause screen (the M9 crossbar shows while it
//! loads and when it is missing or failed), with the Vita's layout and settings (`vita_menus`).
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use skate_ui::fe::menu_stage::{MenuStage, APT_START, PAUSE_ROOTS};
use skate_ui::fe::retail_menu_movies::RetailMenuMovies;
use vita_render::apt::Apt;

use crate::packs::ROOT;
use crate::settings::Settings;
use crate::vita_menus::{Choice, Worlds};

/// The movie set in ui.pak (vita-bake `MENU_MOVIES`).
const SET: &str = "private/menu-movies";
/// The living world's skater profiles (living.pak): the pros Call Skater lists.
const PROFILES: &str = "private/living_world/skater_profiles.json";
/// Textures uploaded per frame while the set settles.
const UPLOADS_PER_FRAME: usize = 4;
/// Seconds the stage keeps running and drawing after the stack empties: the closed screen's outro plays out
/// (upstream updates the stage every frame) and the next open finds it unloaded.
const CLOSING_SECONDS: f32 = 2.0;

/// A texture to upload: its key (the path in the set), size and RGBA bytes.
type Texture = (String, u32, u32, Vec<u8>);

pub struct Loaded {
    stage: MenuStage,
    textures: Vec<Texture>,
    worlds: Worlds,
}

/// What a frame of the open menu asks of the game.
#[derive(Default)]
pub struct Update {
    /// Still open (false: the player closed it, back to playing).
    pub open: bool,
    pub choices: Vec<Choice>,
    /// The player changed a setting (the game's settings were updated).
    pub settings_changed: bool,
    /// Challenge Map locations picked: their map (`packs::WORLDS` name) and locator matrix.
    pub teleports: Vec<(String, [[f32; 4]; 4])>,
    /// Call Skater (FE 62): a character key to call (true) or dismiss (false), in order.
    pub skater_calls: Vec<(String, bool)>,
}

pub enum FeMenus {
    /// Waiting for ui.pak.
    Idle,
    /// No movie set in ui.pak.
    Absent,
    Loading(Receiver<Result<Loaded, String>>),
    Ready(Box<Ready>),
    Failed(String),
}

pub struct Ready {
    stage: MenuStage,
    pending: Vec<Texture>,
    /// A screen of ours is open (from `open` until the stack empties).
    shown: bool,
    /// The outro is not drawn (the game moved to another screen).
    hidden: bool,
    /// Named images uploaded on first use (the Challenge Map's world maps and marker icons), freed once the menus
    /// have closed.
    named: Vec<String>,
    worlds: Worlds,
    /// Seconds left of the closing outro.
    closing: f32,
    /// The retail loading screen (FE 52) runs for this load (`set_loading`), and whether the game still loads.
    loading_screen: bool,
    load_wanted: bool,
    /// The retail replay screen is open (`FeMenus::replay`), and the choices made in its popups since last taken.
    replay: bool,
    replay_choices: Vec<(String, i32)>,
    /// The stage runs over the game (the teleporter prompt or its confirm popup; `hud`).
    hud: bool,
    /// The system keyboard the text entry (Extras > Enter Cheat Code) opens.
    #[cfg(target_os = "vita")]
    keyboard: vita_platform::ime::Keyboard,
    stats: Stats,
}

/// Frame cost while shown, printed every 2 s.
#[derive(Default)]
struct Stats {
    frames: u32,
    update_ms: f32,
    draw_ms: f32,
    worst_ms: f32,
    draws: usize,
    since: Option<Instant>,
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    skate_data::vfs::read(&Path::new(ROOT).join(path)).map_err(|e| format!("{path}: {e}"))
}

fn heap() -> usize {
    crate::alloc::large_blocks().0 + crate::alloc::small_blocks()
}

fn load(settings: Settings, worlds: Worlds) -> Result<Loaded, String> {
    let started = Instant::now();
    let heap_before = heap();
    let read_set = |p: &str| read(&format!("{SET}/{p}"));
    let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS))?;
    let set_ms = started.elapsed().as_secs_f32() * 1000.0;
    for (key, e) in &movies.failures {
        println!("FE_MENUS movie {key}: {e}");
    }
    let count = movies.movies.len();
    let wanted: Vec<(String, [u32; 2])> = movies.retail_images().map(|(k, s)| (k.to_owned(), s)).collect();
    let booted = Instant::now();
    let mut stage = MenuStage::boot_with(movies, &read, &mut |c| crate::vita_menus::customize(c, &worlds))?;
    crate::vita_menus::register(&mut stage.menus.registry, &settings);
    // Challenge Map locations in the worlds the Vita has can be travelled to; the others stay greyed.
    let travel = stage.register_teleports(&|map| crate::packs::WORLDS.contains(&map));
    println!("FE_MENUS challenge map: {travel} locations to travel to");
    // Call Skater lists the pros of the living world's skater profiles (living.pak); none without it.
    let callable = read(PROFILES).map(|b| skate_ui::fe::spawn_skater_host::callable(&b)).unwrap_or_default();
    println!("FE_MENUS call skater: {} pros", callable.len());
    stage.set_callable(callable, crate::settings::CALLED_SKATERS as usize);
    let boot_ms = booted.elapsed().as_secs_f32() * 1000.0;
    let heap_stage = heap();
    let mut textures = Vec::with_capacity(wanted.len());
    let mut bytes_total = 0;
    for (key, [w, h]) in wanted {
        let bytes = read(&format!("{SET}/{key}"))?;
        if bytes.len() != w as usize * h as usize * 4 {
            return Err(format!("menu texture {key}: {} bytes, the set says {w} x {h}", bytes.len()));
        }
        bytes_total += bytes.len();
        textures.push((key, w, h, bytes));
    }
    println!(
        "FE_MENUS loaded in {:.0} ms: {count} movies (set {set_ms:.0} ms, boot {boot_ms:.0} ms), {} textures {} KiB; heap +{} KiB for the stage ({}), +{} KiB with the textures",
        started.elapsed().as_secs_f32() * 1000.0,
        textures.len(),
        bytes_total / 1024,
        (heap_stage as isize - heap_before as isize) / 1024,
        stage.object_summary(),
        (heap() as isize - heap_before as isize) / 1024,
    );
    Ok(Loaded { stage, textures, worlds })
}

impl FeMenus {
    /// Starts loading once ui.pak is mounted (after the other UI loaders: they slow each other down).
    pub fn start(&mut self, settings: Settings, worlds: Worlds) {
        if !matches!(self, FeMenus::Idle) {
            return;
        }
        if !crate::packs::UI.load(std::sync::atomic::Ordering::Relaxed)
            || !skate_data::vfs::is_file(&Path::new(ROOT).join(SET).join("manifest.json"))
        {
            println!("FE_MENUS: no menu movie set in ui.pak");
            *self = FeMenus::Absent;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("fe-load".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                crate::alloc::tag_thread("load");
                let _ = vita_platform::vita::pin_current_thread(2);
                let _ = tx.send(load(settings, worlds));
            })
            .expect("spawn the front-end loader");
        *self = FeMenus::Loading(rx);
    }

    /// Loaded and every texture uploaded.
    pub fn ready(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.pending.is_empty() && !r.stage.failed)
    }

    pub fn failure(&self) -> Option<&str> {
        match self {
            FeMenus::Failed(e) => Some(e),
            _ => None,
        }
    }

    pub fn idle(&self) -> bool {
        matches!(self, FeMenus::Idle)
    }

    /// Still on its way (waiting for ui.pak, loading, or uploading textures): an open asked now waits for it.
    pub fn loading(&self) -> bool {
        match self {
            FeMenus::Idle | FeMenus::Loading(_) => true,
            FeMenus::Ready(r) => !r.pending.is_empty(),
            FeMenus::Absent | FeMenus::Failed(_) => false,
        }
    }

    fn fail(&mut self, error: String) {
        println!("FE_MENUS off: {error}");
        *self = FeMenus::Failed(error);
    }

    /// Once per frame: finishes loading, uploads textures, plays a closing outro.
    pub fn frame(&mut self, apt: &mut Apt, seconds: f32) {
        if let FeMenus::Loading(rx) = self {
            match rx.try_recv() {
                Ok(Ok(loaded)) => {
                    *self = FeMenus::Ready(Box::new(Ready {
                        stage: loaded.stage,
                        pending: loaded.textures,
                        shown: false,
                        hidden: false,
                        named: Vec::new(),
                        worlds: loaded.worlds,
                        closing: 0.0,
                        replay: false,
                        replay_choices: Vec::new(),
                        hud: false,
                        loading_screen: false,
                        load_wanted: false,
                        #[cfg(target_os = "vita")]
                        keyboard: Default::default(),
                        stats: Stats::default(),
                    }))
                }
                Ok(Err(e)) => self.fail(e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.fail("the loader stopped".into()),
            }
        }
        if let FeMenus::Ready(r) = self {
            for _ in 0..UPLOADS_PER_FRAME {
                let Some((key, w, h, bytes)) = r.pending.pop() else { break };
                apt.add_texture(&key, w, h, &bytes);
                if r.pending.is_empty() {
                    println!("FE_MENUS textures uploaded; heap {} KiB", heap() / 1024);
                }
            }
            // The closing outro and the loading screen share the one update a frame.
            let closing = !r.shown && r.closing > 0.0;
            if closing || r.loading_screen {
                if closing {
                    r.closing -= seconds;
                }
                let started = Instant::now();
                let frame = r.stage.update_keys(Vec::new(), seconds);
                if r.loading_screen {
                    let ms = started.elapsed().as_secs_f32() * 1000.0;
                    r.stats.worst_ms = r.stats.worst_ms.max(ms);
                }
                for line in &frame.log {
                    println!("FE_MENUS {line}");
                }
                if r.stage.failed {
                    let error = frame.log.last().cloned().unwrap_or_default();
                    self.fail(error);
                    return;
                }
                if r.loading_screen && !r.load_wanted && !r.stage.load_active() {
                    r.loading_screen = false;
                    println!("FE_MENUS loading screen done; heap {} KiB", heap() / 1024);
                }
                if closing && r.closing <= 0.0 {
                    for key in r.named.drain(..) {
                        apt.remove_texture(&key);
                    }
                    println!("FE_MENUS closed; heap {} KiB, {}", heap() / 1024, r.stage.object_summary());
                }
            }
        }
    }

    /// The retail loading screen (FE 52) around a world load: `loading` from the load's start until the world is
    /// ready, `began` on the frame it starts. A load that began before the stage could show it keeps the plain
    /// loading screen (the first world's: the menus load after it).
    pub fn set_loading(&mut self, loading: bool, began: bool) {
        let FeMenus::Ready(r) = self else { return };
        if !r.stage.has_loading_screen() || r.stage.failed || !r.pending.is_empty() {
            return;
        }
        if loading && began && !r.loading_screen {
            r.loading_screen = true;
            println!("FE_MENUS loading screen; heap {} KiB", heap() / 1024);
        }
        r.load_wanted = loading;
        if r.loading_screen {
            r.stage.set_loading(loading);
        }
    }

    /// The career stats the loading screen's Stats panel shows from its next open, and the Units setting.
    pub fn set_career_stats(&mut self, stats: &skate_core::career_stats::CareerStats, imperial: bool) {
        if let FeMenus::Ready(r) = self {
            r.stage.set_career_stats(stats, imperial);
        }
    }

    /// The loading screen runs (the menus draw it over everything).
    pub fn loading_screen(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.loading_screen)
    }

    /// The loading screen covers the world (the fader black or FE 52 up): black under the menus, not the world.
    pub fn loading_covered(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.loading_screen && r.stage.load_covered())
    }

    /// Opens the pause menu (retail Start), or the main menu, with the game's current settings and saved replays.
    pub fn open(&mut self, settings: &Settings, replays: &[String], main: bool) {
        if let FeMenus::Ready(r) = self {
            if !r.shown {
                crate::vita_menus::set_main_menu(&mut r.stage.menus.context, main);
                r.stage.closable = !main;
                crate::vita_menus::write_settings(&mut r.stage.menus.registry, settings);
                crate::vita_menus::set_facts(&mut r.stage.menus.context, settings);
                r.stage.set_sub_options(crate::vita_menus::REPLAY_KIND, crate::vita_menus::replay_row(replays));
                r.shown = true;
                r.hidden = false;
                r.closing = 0.0;
                r.stats = Stats::default();
                println!("FE_MENUS open; heap {} KiB", heap() / 1024);
                r.stage.update_keys(vec![(APT_START, 0)], 0.0);
            }
        }
    }

    /// The Trick Guide's demo while its screen is on top: (clip, camera track).
    pub fn trick_guide_demo(&self) -> Option<(String, i16)> {
        match self {
            FeMenus::Ready(r) if r.shown => r.stage.trick_guide_demo().map(|(a, t)| (a.to_owned(), t)),
            _ => None,
        }
    }

    /// The game's replay this frame (None: no replay open): the retail replay screen (FE 12, its help FE 13) on the
    /// menu stage, opened, fed and closed with it. True while that screen takes the replay (else the game's own
    /// replay screens show).
    /// `pad`: the 24 native values; they go to the stage only while a replay popup is shown (the Finished menu).
    pub fn replay(&mut self, feed: Option<skate_ui::fe::replay_menu_host::ReplayScreenFeed>, seconds: f32, pad: &[f32; 24]) -> bool {
        let FeMenus::Ready(r) = self else { return false };
        if !r.stage.has_replay() || r.stage.failed || !r.pending.is_empty() || (feed.is_none() && !r.replay) {
            return false;
        }
        r.replay = feed.is_some();
        if !r.replay {
            // The replay closed: its popups go with it.
            r.stage.remove_popups(REPLAY_POPUP);
            r.replay_choices.clear();
        }
        r.stage.set_replay(feed);
        // The pads always go through the stage's press tracking (else the press that raised the Finished popup reads as
        // new on its first frame and cancels it); their keys count only while a popup is shown.
        let frame = if r.stage.popup_shown() {
            r.stage.update(std::slice::from_ref(pad), seconds)
        } else {
            r.stage.track_pads(std::slice::from_ref(pad), seconds);
            r.stage.update_keys(Vec::new(), seconds)
        };
        r.replay_choices.extend(frame.popup_choices.iter().filter(|(c, _)| c == REPLAY_POPUP).cloned());
        for line in &frame.log {
            println!("FE_MENUS {line}");
        }
        if r.stage.failed {
            let error = frame.log.last().cloned().unwrap_or_default();
            self.fail(error);
            return false;
        }
        r.replay
    }

    /// Raises the replay's Finished menu (Upload Video, Save Replay, Exit; B closes it) as the stage's popup.
    pub fn replay_popup(&mut self, options: &[&str]) {
        if let FeMenus::Ready(r) = self {
            r.stage.push_popup(skate_core::fe_popup::PopupRequest {
                // P1's Finished menu: the list alone (NOT RETAIL YET: retail's title for it is not found).
                title: String::new(),
                description: String::new(),
                options: options.iter().map(|o| (*o).to_owned()).collect(),
                cancelable: true,
                callback: REPLAY_POPUP.into(),
                ..Default::default()
            });
        }
    }

    /// The option chosen in the replay's popup since the last call (None: none; B: a negative or 255).
    pub fn take_replay_choice(&mut self) -> Option<i32> {
        match self {
            FeMenus::Ready(r) => r.replay_choices.pop().map(|(_, c)| c),
            _ => None,
        }
    }

    /// The action overlay is in this install (the teleporters' prompt).
    pub fn has_signup(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.stage.has_signup() && r.pending.is_empty() && !r.stage.failed)
    }

    /// Over the game, once a frame: the teleporter prompt (`signup`: its title, None hides it) and the confirm popup
    /// to raise; the stage runs only while either is up (the pad goes to the popup). Returns the confirm's choice
    /// (`teleporters::CONFIRM`; B: negative or 255). While the menus or the replay screen are open, only the prompt's
    /// title is passed on (their update posts it).
    pub fn hud(&mut self, signup: Option<String>, popup: Option<skate_core::fe_popup::PopupRequest>, seconds: f32, pad: &[f32; 24]) -> Option<i32> {
        let FeMenus::Ready(r) = self else { return None };
        if !r.stage.has_signup() || r.stage.failed || !r.pending.is_empty() {
            return None;
        }
        r.stage.set_signup(signup);
        if r.shown || r.replay {
            r.hud = false;
            return None;
        }
        if let Some(popup) = popup {
            r.stage.push_popup(popup);
            r.hud = true;
        }
        r.hud = r.hud && r.stage.popup_shown() || r.stage.signup_active();
        if !r.hud {
            return None;
        }
        let frame = if r.stage.popup_shown() {
            r.stage.update(std::slice::from_ref(pad), seconds)
        } else {
            r.stage.track_pads(std::slice::from_ref(pad), seconds);
            r.stage.update_keys(Vec::new(), seconds)
        };
        for line in &frame.log {
            println!("FE_MENUS {line}");
        }
        if r.stage.failed {
            let error = frame.log.last().cloned().unwrap_or_default();
            self.fail(error);
            return None;
        }
        // A raised popup shows on the next update: keep running until it is shown and answered.
        r.hud |= r.stage.popup_shown();
        frame.popup_choices.iter().filter(|(c, _)| c == crate::teleporters::CONFIRM).map(|(_, choice)| *choice).last()
    }

    /// The system keyboard is up (the frame swap lets it draw).
    pub fn keyboard_running(&self) -> bool {
        #[cfg(target_os = "vita")]
        if let FeMenus::Ready(r) = self {
            return r.keyboard.running();
        }
        false
    }

    /// The retail replay screen is open on the stage (it closes with `replay(None, ..)`).
    pub fn replay_open(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.replay)
    }

    /// The retail replay screen is shown (its intro over); its help over it.
    pub fn replay_shown(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.replay && r.stage.replay_shown())
    }

    /// A screen of ours is open.
    pub fn shown(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.shown)
    }

    /// Closes it at once: the game moves to another screen (no outro drawn over it).
    pub fn close_now(&mut self) {
        if let FeMenus::Ready(r) = self {
            if r.shown {
                r.stage.close();
                r.shown = false;
                r.hidden = true;
                r.closing = CLOSING_SECONDS;
            }
        }
    }

    /// Closes it with its outro over the game (back to playing): the outro plays in the closing frames (`frame`),
    /// which run whatever the screen is.
    pub fn close(&mut self) {
        if let FeMenus::Ready(r) = self {
            if r.shown {
                r.stage.close();
                r.shown = false;
                r.hidden = false;
                r.closing = CLOSING_SECONDS;
            }
        }
    }

    /// The on-screen help rows follow the settings (their labels offer "off" for what is on).
    pub fn set_facts(&mut self, settings: &Settings) {
        if let FeMenus::Ready(r) = self {
            crate::vita_menus::set_facts(&mut r.stage.menus.context, settings);
        }
    }

    /// One frame while shown, with the pad's 24 native values: choices and settings changes for the game (the
    /// settings rows the player changed are written to `settings`).
    pub fn update(&mut self, pad: &[f32; 24], seconds: f32, settings: &mut Settings, called: &[String]) -> Update {
        let FeMenus::Ready(r) = self else { return Update::default() };
        if !r.shown {
            return Update::default();
        }
        let mut out = Update { open: true, ..Update::default() };
        r.stage.set_cheats(if settings.zombie { &["zombie"] } else { &[] });
        r.stage.set_called(called);
        r.stage.set_free_play(settings.free_play());
        // The system keyboard takes the pad while it is up; its answer goes to the text entry.
        #[cfg(target_os = "vita")]
        let typing = r.keyboard.running();
        #[cfg(not(target_os = "vita"))]
        let typing = false;
        #[cfg(target_os = "vita")]
        if let Some(text) = r.keyboard.poll() {
            r.stage.text_entered(text);
        }
        let pad = if typing { &[0.0; 24] } else { pad };
        let started = Instant::now();
        let frame = r.stage.update(std::slice::from_ref(pad), seconds);
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        r.stats.update_ms += ms;
        r.stats.worst_ms = r.stats.worst_ms.max(ms);
        for line in &frame.log {
            println!("FE_MENUS {line}");
        }
        let picks = frame.items.iter().map(|(item, _)| (item.as_str(), None)).chain(frame.sub_options.iter().map(|(item, sub)| (item.as_str(), Some(*sub))));
        for (item, sub) in picks {
            match crate::vita_menus::choice(item, sub, &r.worlds) {
                Some(choice) => out.choices.push(choice),
                None => println!("FE_MENUS item {item} {sub:?}: nothing on the Vita"),
            }
        }
        out.teleports.extend(frame.teleports.iter().filter_map(|key| r.stage.teleport(key).cloned()));
        if let Some((title, request)) = &frame.text_entry {
            #[cfg(target_os = "vita")]
            let opened = r.keyboard.open(title, request.max_len);
            #[cfg(not(target_os = "vita"))]
            let opened = false;
            println!("FE_MENUS keyboard for {} ({title:?}): {}", request.callback, if opened { "open" } else { "refused" });
            if !opened {
                r.stage.text_entered(None);
            }
        }
        for (id, on) in &frame.cheats {
            println!("FE_MENUS cheat {id} {}", if *on { "on" } else { "off" });
            if id == "zombie" {
                settings.zombie = *on;
                out.settings_changed = true;
            }
        }
        // Call Skater: calls and dismisses go to the sim's living world; Free Play's You (the only skater listed: no
        // player character swap on the Vita) changes nothing.
        for action in &frame.spawn_skater {
            use skate_core::spawn_skater::{Action, Skater};
            match action {
                Action::Call(Skater::Character(key)) => out.skater_calls.push((key.clone(), true)),
                Action::Dismiss(Skater::Character(key)) => out.skater_calls.push((key.clone(), false)),
                _ => {}
            }
        }
        // The Free Play options (FE 34): its values to the Vita's Traffic / Pedestrians levels and AI skaters.
        if let Some(values) = frame.free_play {
            if settings.set_free_play(values) {
                crate::vita_menus::write_settings(&mut r.stage.menus.registry, settings);
                out.settings_changed = true;
            }
        }
        for (row, _) in &frame.settings {
            out.settings_changed |= crate::vita_menus::read_setting(&r.stage.menus.registry, row, settings);
        }
        if r.stage.failed {
            let error = frame.log.last().cloned().unwrap_or_default();
            self.fail(error);
            out.open = false;
            return out;
        }
        if !r.stage.open() && !r.stage.suspended() {
            r.shown = false;
            r.closing = CLOSING_SECONDS;
            out.open = false;
        }
        out
    }

    /// Draws the menus; returns the controller glyphs the movies leave to the host (named as the replay screens'
    /// glyphs, `app::draw_glyphs`: retail name with `_hud`, rectangle in stage space).
    pub fn draw(&mut self, apt: &mut Apt) -> Vec<(String, [f32; 4])> {
        let FeMenus::Ready(r) = self else { return Vec::new() };
        if !r.shown && !r.replay && !r.hud && !r.loading_screen && (r.closing <= 0.0 || r.hidden) {
            return Vec::new();
        }
        let started = Instant::now();
        let draws = r.stage.draw();
        // Named images (image/...: the Challenge Map's world maps, its marker icons) upload when first drawn.
        for d in draws.iter().filter(|d| d.texture.starts_with(skate_ui::fe::retail_menu_movies::NAMED_IMAGE_PREFIX)) {
            if apt.has_texture(&d.texture) || r.named.contains(&d.texture) {
                continue;
            }
            r.named.push(d.texture.clone());
            let Some(skate_ui::fe::retail_menu_movies::MenuImage::Retail { path, width, height }) = r.stage.image(&d.texture) else { continue };
            match read(&format!("{SET}/{path}")) {
                Ok(bytes) if bytes.len() == width as usize * height as usize * 4 => apt.add_texture(&d.texture, width, height, &bytes),
                Ok(bytes) => println!("FE_MENUS image {}: {} bytes, the set says {width} x {height}", d.texture, bytes.len()),
                Err(e) => println!("FE_MENUS image {}: {e}", d.texture),
            }
        }
        apt.draw(&draws);
        let glyphs: Vec<(String, [f32; 4])> = r.stage.prompts().iter().map(|(name, rect)| (glyph_name(name), *rect)).collect();
        let s = &mut r.stats;
        s.draw_ms += started.elapsed().as_secs_f32() * 1000.0;
        s.draws += draws.len();
        s.frames += 1;
        let since = *s.since.get_or_insert_with(Instant::now);
        if since.elapsed().as_secs_f32() >= 2.0 {
            let n = s.frames.max(1) as f32;
            println!(
                "FE_MENUS frame: update {:.2} ms (worst {:.2}), draw {:.2} ms, {:.0} draws; heap {} KiB",
                s.update_ms / n,
                s.worst_ms,
                s.draw_ms / n,
                s.draws as f32 / n,
                heap() / 1024
            );
            // `fe-profile`: ms per frame of each phase of the front end (nested: a phase includes those inside it).
            let spans: Vec<String> =
                skate_ui::fe::profile::take().iter().map(|(name, ms, _, _)| format!("{name}={:.2}", ms / f64::from(n))).collect();
            if !spans.is_empty() {
                println!("FE_MENUS spans: {}", spans.join(" "));
            }
            *s = Stats { since: Some(Instant::now()), ..Stats::default() };
        }
        glyphs
    }
}

/// The callback of the replay's popups (the Finished menu).
const REPLAY_POPUP: &str = "replay.finished";

/// A retail prompt's button as the replay screens name their glyphs: the face buttons as they are, the others with
/// `_hud` (a stick or the D-pad without a direction suffix: the stick, the D-pad's arrows).
fn glyph_name(retail: &str) -> String {
    // Already the HUD-style name (the replay screen's prompts: `L3_Click_hud`, `DPad_UpDown_hud`).
    if retail.ends_with("_hud") || matches!(retail, "A" | "B" | "X" | "Y") {
        return retail.to_owned();
    }
    for (prefix, plain) in [("Stick_Left", "Stick_Left"), ("Stick_Right", "Stick_Right")] {
        if retail.starts_with(prefix) {
            return format!("{plain}_hud");
        }
    }
    match retail {
        "DPad" => "DPad_Arrows_hud".to_owned(),
        "DPad_L" => "DPad_Left_hud".to_owned(),
        "DPad_R" => "DPad_Right_hud".to_owned(),
        other => format!("{other}_hud"),
    }
}
