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
    pub teleports: Vec<(String, String, [[f32; 4]; 4])>,
    /// Call Skater (FE 62): a character key to call (true) or dismiss (false), in order.
    pub skater_calls: Vec<(String, bool)>,
    /// Unlocks granted (Extras codes), for the unlock store.
    pub unlocks: Vec<(skate_core::unlocks::UnlockKind, String)>,
    /// The skater the player left the customisation with.
    pub cac: Option<skate_core::skater_record::SkaterRecord>,
    /// Text the game asked for and the player typed: (callback, text) (Save Park's name).
    pub texts: Vec<(String, String)>,
    /// The game's own popups answered: (callback, option) (Save Park's overwrite question).
    pub answers: Vec<(String, i32)>,
    /// My Parks: the saved park the player selected or deleted (its index in the list given).
    pub browser: Vec<skate_ui::fe::browser_host::BrowserEvent>,
    /// New Park: the lot and template the player took in Park Setup.
    pub park_setup: Vec<skate_ui::fe::park_setup_host::ParkSetupChoice>,
    /// What the team screen asks for.
    pub team: Vec<skate_core::team::Effect>,
}

/// Save Park's text entry and popup callbacks.
pub const PARK_NAME: &str = "park.name";
/// The team screen's keyboards: the team's name, a member's (the member's index after the dot).
pub const TEAM_NAME: &str = "team.name";
pub const MEMBER_NAME: &str = "team.member.";
pub const PARK_OVERWRITE: &str = "park.overwrite";
/// My Parks' question before loading over unsaved edits.
pub const PARK_LOAD: &str = "park.load";
/// Park Setup's question before laying a template over unsaved edits.
pub const PARK_DIRTY: &str = "park.dirty";
/// My Parks' question before deleting a saved park.
pub const PARK_DELETE: &str = "park.delete";

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
    /// Slappy's thumbnails on the GPU (`cas.pak` `cas/thumbs/`), freed as soon as they are not drawn, and the ones the
    /// pack lacks (not asked again).
    thumbs: Vec<String>,
    thumbs_missing: Vec<String>,
    worlds: Worlds,
    /// Seconds left of the closing outro.
    closing: f32,
    /// The retail loading screen (FE 52) runs for this load (`set_loading`), and whether the game still loads.
    loading_screen: bool,
    load_wanted: bool,
    /// The retail replay screen is open (`FeMenus::replay`), and the choices made in its popups since last taken.
    replay: bool,
    replay_choices: Vec<(String, i32)>,
    /// The Object Dropper's retail screen is open (`FeMenus::object_dropper`), and its list callbacks for the editor.
    dropper: bool,
    dropper_list: Vec<(usize, bool)>,
    /// The stage runs over the game (the teleporter prompt or its confirm popup; `hud`).
    hud: bool,
    /// The system keyboard the text entry (Extras > Enter Cheat Code) opens.
    #[cfg(target_os = "vita")]
    keyboard: vita_platform::ime::Keyboard,
    /// The keyboard is up for the game itself (its callback), not for the stage's text entry.
    own_entry: Option<String>,
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
                        thumbs: Vec::new(),
                        thumbs_missing: Vec::new(),
                        worlds: loaded.worlds,
                        closing: 0.0,
                        replay: false,
                        replay_choices: Vec::new(),
                        dropper: false,
                        dropper_list: Vec::new(),
                        hud: false,
                        loading_screen: false,
                        load_wanted: false,
                        #[cfg(target_os = "vita")]
                        keyboard: Default::default(),
                        own_entry: None,
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
                    for key in r.named.drain(..).chain(r.thumbs.drain(..)) {
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
    pub fn open(&mut self, settings: &Settings, replays: &[String], main: bool, park: bool) {
        if let FeMenus::Ready(r) = self {
            if !r.shown {
                // The pause menu over a park editor: the editor closed first (825CB440); its screen goes now, its outro
                // run by the menu's own updates (device: it stayed drawn under the menu).
                if r.dropper {
                    r.stage.set_object_dropper(None, &[], &[]);
                    r.stage.remove_popups(crate::dropper::RESET_POPUP);
                    r.dropper = false;
                }
                crate::vita_menus::set_main_menu(&mut r.stage.menus.context, main, park);
                r.stage.closable = !main;
                crate::vita_menus::write_settings(&mut r.stage.menus.registry, settings);
                crate::vita_menus::set_facts(&mut r.stage.menus.context, settings);
                r.stage.set_sub_options(crate::vita_menus::REPLAY_KIND, crate::vita_menus::replay_row(replays));
                r.shown = true;
                r.hidden = false;
                r.closing = 0.0;
                r.stats = Stats::default();
                println!("FE_MENUS open; heap {} KiB, largest free block {} MiB", heap() / 1024, crate::alloc::largest_free_mib());
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

    /// The Object Dropper's retail screen (template ObjectDropper) on the menu stage: opened, fed and closed with the
    /// editor (`feed` None: closed), the editor's screen modes posted; `ask_reset` raises the open world's reset
    /// question (`ID_DMO_RESET_CONFIRMATION`, Yes / No, retail `825C6548`), which takes the pad until answered.
    /// Returns the question's answer (0 Yes, 1 No; B: negative or 255).
    pub fn object_dropper(
        &mut self,
        feed: Option<skate_ui::fe::object_dropper_host::ObjectDropperFeed>,
        posts: &[u8],
        quick_posts: &[u8],
        ask_reset: bool,
        seconds: f32,
        pad: &[f32; 24],
    ) -> Option<i32> {
        let FeMenus::Ready(r) = self else { return None };
        // After the editor closed, the stage keeps running until the screen's outro unloads it (else it stays drawn
        // under the next menu).
        let closing = feed.is_none() && !r.dropper && !r.shown && r.stage.object_dropper_loaded();
        if !r.stage.has_object_dropper() || r.stage.failed || !r.pending.is_empty() || (feed.is_none() && !r.dropper && !closing) {
            return None;
        }
        r.dropper = feed.is_some();
        if !r.dropper {
            r.stage.remove_popups(crate::dropper::RESET_POPUP);
        }
        // The Quick Menu's Style / Branding pages: the movie's list takes the pad (`mbAllowMenuPickerInput`).
        use skate_ui::fe::object_dropper_host::{PAGE_BRANDING, PAGE_STYLE};
        let list = feed.as_ref().and_then(|f| f.quick.as_ref()).is_some_and(|q| q.page == PAGE_STYLE || q.page == PAGE_BRANDING);
        r.stage.set_object_dropper(feed, posts, quick_posts);
        if ask_reset && r.dropper {
            r.stage.push_popup(skate_core::fe_popup::PopupRequest {
                title: "ID_COMMON_WARNING".into(),
                description: "ID_DMO_RESET_CONFIRMATION".into(),
                options: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
                cancelable: true,
                callback: crate::dropper::RESET_POPUP.into(),
                ..Default::default()
            });
        }
        let frame = if r.stage.popup_shown() || list {
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
        r.dropper_list.extend(r.stage.take_object_dropper_list_calls());
        frame.popup_choices.iter().filter(|(c, _)| c == crate::dropper::RESET_POPUP).map(|(_, choice)| *choice).last()
    }

    /// The Quick Menu's Style / Branding list callbacks since the last call: (row, selected).
    pub fn take_dropper_list(&mut self) -> Vec<(usize, bool)> {
        match self {
            FeMenus::Ready(r) => std::mem::take(&mut r.dropper_list),
            _ => Vec::new(),
        }
    }

    /// The HUD phone's skate.Park rows: the font and strings (None until the menus have loaded).
    pub fn phone_text(&self) -> Option<(&skate_ui::fe::apt_text::Font, &str, &str)> {
        match self {
            FeMenus::Ready(r) => r.stage.phone_text(),
            _ => None,
        }
    }

    /// The Object Dropper's movie is still on the menu stage (open, or playing its outro).
    pub fn object_dropper_loaded(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.dropper || r.stage.object_dropper_loaded())
    }

    /// The Object Dropper's retail screen is open.
    pub fn object_dropper_open(&self) -> bool {
        matches!(self, FeMenus::Ready(r) if r.dropper)
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

    /// Save Park's name (retail `8270A278`: the keyboard titled `ID_SKATEPARK_NAME_PARK`, the park's name typed, 16
    /// characters at most) over the menu; the answer comes in [`Update::texts`] under [`PARK_NAME`]. NOT RETAIL YET:
    /// the system keyboard opens over the pause menu itself (retail pushes FE 0x2E; the stage's FE 46 closed the menu).
    pub fn ask_park_name(&mut self, name: &str) {
        self.ask_text(PARK_NAME, "ID_SKATEPARK_NAME_PARK", name, 16);
    }

    /// The system keyboard titled `title_id` on `text` (at most `max` characters); the answer comes in
    /// [`Update::texts`] under `callback`. The team screen's names use it (retail 826397D8 pushes FE 0x2E).
    pub fn ask_text(&mut self, callback: &str, title_id: &str, text: &str, max: usize) {
        if let FeMenus::Ready(r) = self {
            #[cfg(target_os = "vita")]
            {
                let title = r.stage.text(title_id);
                let opened = r.keyboard.open_with(&title, text, max);
                println!("FE_MENUS keyboard for {callback} ({title:?}): {}", if opened { "open" } else { "refused" });
                if opened {
                    r.own_entry = Some(callback.into());
                }
            }
            #[cfg(not(target_os = "vita"))]
            let _ = (r, callback, title_id, text, max);
        }
    }

    /// Opens the team screen (FE 67); false without its movie.
    pub fn open_team(&mut self, team: skate_core::team::Team) -> bool {
        match self {
            FeMenus::Ready(r) => r.stage.open_team(team),
            _ => false,
        }
    }

    /// A name typed for the team screen.
    pub fn team_named(&mut self, member: Option<usize>, text: &str) {
        if let FeMenus::Ready(r) = self {
            r.stage.team_named(member, text);
        }
    }

    /// A taken name (retail `8270A398`): `ID_COMMON_WARNING_TITLE` / `ID_SKATEPARK_ALREADY_EXISTS`, Yes (0) overwrites, No
    /// (1) goes back to the keyboard; the answer comes in [`Update::answers`] under [`PARK_OVERWRITE`].
    pub fn ask_park_overwrite(&mut self) {
        if let FeMenus::Ready(r) = self {
            r.stage.push_popup(skate_core::fe_popup::PopupRequest {
                title: "ID_COMMON_WARNING_TITLE".into(),
                description: "ID_SKATEPARK_ALREADY_EXISTS".into(),
                options: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
                cancelable: true,
                callback: PARK_OVERWRITE.into(),
                ..Default::default()
            });
        }
    }

    /// My Parks (skate.Park's second row): retail's browser over the menu with the saved parks; false without its movie.
    pub fn open_parks(&mut self, items: Vec<skate_ui::fe::browser_host::BrowserItem>) -> bool {
        match self {
            FeMenus::Ready(r) => r.stage.open_browser(items),
            _ => false,
        }
    }

    /// New Park (skate.Park's first row): retail's Park Setup over the menu with the lots; false without its movie.
    /// The character customisation (FE 15) on a skater; false without its movie.
    /// The customisation's skater when its mesh is out of date (see `MenuStage::cac_look`).
    pub fn cac_look(&mut self) -> Option<(skate_core::skater_record::SkaterRecord, skate_core::cac::LookChange)> {
        match self {
            FeMenus::Ready(r) => r.stage.cac_look(),
            _ => None,
        }
    }

    pub fn cac_decal_rows(&self) -> Option<[[[f32; 3]; 2]; 5]> {
        match self {
            FeMenus::Ready(r) => r.stage.cac_decal_rows(),
            _ => None,
        }
    }

    pub fn cac_look_done(&mut self) {
        if let FeMenus::Ready(r) = self {
            r.stage.cac_look_done();
        }
    }

    /// What the customisation's preview draws while it is open.
    pub fn cac_preview(&self) -> Option<skate_core::cac::Preview> {
        match self {
            FeMenus::Ready(r) if r.shown => r.stage.cac_preview(),
            _ => None,
        }
    }

    pub fn open_cac(&mut self, screen: skate_core::cac::Cac, cash: u32) -> bool {
        match self {
            FeMenus::Ready(r) => r.stage.open_cac(screen, cash),
            _ => false,
        }
    }

    pub fn open_park_setup(&mut self, lots: Vec<skate_ui::fe::park_setup_host::ParkSetupLot>) -> bool {
        match self {
            FeMenus::Ready(r) => r.stage.open_park_setup(lots),
            _ => false,
        }
    }

    /// Laying a template over unsaved edits (retail `825D2588`): `ID_SKATEPARK_DIRTY_TITLE` / `ID_SKATEPARK_DIRTY_DESC`,
    /// No (0) / Yes (1); the answer comes in [`Update::answers`] under [`PARK_DIRTY`].
    pub fn ask_park_dirty(&mut self) {
        if let FeMenus::Ready(r) = self {
            r.stage.push_popup(skate_core::fe_popup::PopupRequest {
                title: "ID_SKATEPARK_DIRTY_TITLE".into(),
                description: "ID_SKATEPARK_DIRTY_DESC".into(),
                options: vec!["ID_COMMON_NO".into(), "ID_COMMON_YES".into()],
                cancelable: true,
                callback: PARK_DIRTY.into(),
                ..Default::default()
            });
        }
    }

    /// Deleting a saved park (retail `82523BC0`): `ID_COMMON_WARNING_TITLE` / `ID_MEMCARD_PROMPT_DELETE_SKATEPARK`, No (0) /
    /// Yes (1); the answer comes in [`Update::answers`] under [`PARK_DELETE`].
    pub fn ask_park_delete(&mut self) {
        if let FeMenus::Ready(r) = self {
            r.stage.push_popup(skate_core::fe_popup::PopupRequest {
                title: "ID_COMMON_WARNING_TITLE".into(),
                description: "ID_MEMCARD_PROMPT_DELETE_SKATEPARK".into(),
                options: vec!["ID_COMMON_NO".into(), "ID_COMMON_YES".into()],
                cancelable: true,
                callback: PARK_DELETE.into(),
                ..Default::default()
            });
        }
    }

    /// The save wheel (a park being written or deleted): shown over the menus or the game while on.
    pub fn set_saving(&mut self, on: bool) {
        if let FeMenus::Ready(r) = self {
            r.stage.set_save_tag(on);
        }
    }

    /// The saved parks changed (a delete): the browser shows them.
    pub fn set_parks(&mut self, items: Vec<skate_ui::fe::browser_host::BrowserItem>) {
        if let FeMenus::Ready(r) = self {
            r.stage.set_browser_items(items);
        }
    }

    /// Loading a saved park over unsaved edits (retail `8262A570`): `ID_SKATEPARK_LOAD_PARK_CONFIRM`, Yes (0) / No (1);
    /// the answer comes in [`Update::answers`] under [`PARK_LOAD`].
    pub fn ask_park_load(&mut self) {
        if let FeMenus::Ready(r) = self {
            r.stage.push_popup(skate_core::fe_popup::PopupRequest {
                title: String::new(),
                description: "ID_SKATEPARK_LOAD_PARK_CONFIRM".into(),
                options: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
                cancelable: true,
                callback: PARK_LOAD.into(),
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
        // The save wheel runs the stage over the game too.
        let saving = r.stage.save_tag_active();
        if (!r.stage.has_signup() && !saving) || r.stage.failed || !r.pending.is_empty() {
            return None;
        }
        if r.stage.has_signup() {
            r.stage.set_signup(signup);
        }
        if r.shown || r.replay || r.dropper {
            r.hud = false;
            return None;
        }
        if let Some(popup) = popup {
            r.stage.push_popup(popup);
            r.hud = true;
        }
        r.hud = r.hud && r.stage.popup_shown() || r.stage.signup_active() || saving;
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
            match r.own_entry.take() {
                // The game's own (Save Park's name): to the game, an empty or cancelled one dropped.
                Some(callback) => out.texts.extend(text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).map(|t| (callback, t))),
                None => r.stage.text_entered(text),
            }
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
        out.teleports.extend(frame.teleports.iter().filter_map(|key| r.stage.teleport(key).map(|(map, matrix)| (key.clone(), map.clone(), *matrix))));
        out.answers.extend(frame.popup_choices.iter().filter(|(c, _)| c.starts_with("park.")).cloned());
        out.browser.extend(r.stage.take_browser_events());
        out.park_setup.extend(r.stage.take_park_setup_choices());
        if let Some((title, request)) = &frame.text_entry {
            #[cfg(target_os = "vita")]
            let opened = r.keyboard.open_with(title, &request.default, request.max_len);
            #[cfg(not(target_os = "vita"))]
            let opened = false;
            println!("FE_MENUS keyboard for {} ({title:?}): {}", request.callback, if opened { "open" } else { "refused" });
            if !opened {
                r.stage.text_entered(None);
            }
        }
        out.unlocks.extend(frame.unlocks.iter().cloned());
        out.cac = r.stage.take_cac();
        out.team.extend(frame.team_effects.iter().cloned());
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
        if !r.shown && !r.replay && !r.dropper && !r.hud && !r.loading_screen && (r.closing <= 0.0 || r.hidden) {
            return Vec::new();
        }
        let started = Instant::now();
        let draws = r.stage.draw();
        // Slappy's thumbnails: a tile's picture is read from cas.pak when it is drawn, at most `THUMBS_PER_FRAME` a
        // frame (16 KiB each, DXT5 as retail's), and freed once no tile draws it: only the shown page stays loaded.
        let mut wanted: Vec<&str> = draws.iter().map(|d| d.texture.as_str()).filter(|t| t.starts_with(THUMB_PREFIX)).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mut loaded = 0;
        for key in &wanted {
            if loaded >= THUMBS_PER_FRAME || apt.has_texture(key) || r.thumbs_missing.iter().any(|m| m == key) {
                continue;
            }
            loaded += 1;
            let path = format!("cas/thumbs/{}.tex", &key[skate_ui::fe::retail_menu_movies::NAMED_IMAGE_PREFIX.len()..]);
            match crate::cas_look::read(&path).and_then(|b| vita_pack::cas::texture_from_pack(&b)) {
                Ok(texture) => {
                    apt.add_compressed_texture(key, &texture);
                    r.thumbs.push((*key).to_owned());
                }
                Err(e) => {
                    println!("FE_MENUS thumbnail {key}: {e}");
                    r.thumbs_missing.push((*key).to_owned());
                }
            }
        }
        r.thumbs.retain(|k| {
            let keep = wanted.binary_search(&k.as_str()).is_ok();
            if !keep {
                apt.remove_texture(k);
            }
            keep
        });
        // Named images (image/...: the Challenge Map's world maps, its marker icons) upload when first drawn.
        for d in draws.iter().filter(|d| d.texture.starts_with(skate_ui::fe::retail_menu_movies::NAMED_IMAGE_PREFIX)) {
            if apt.has_texture(&d.texture) || r.named.contains(&d.texture) || d.texture.starts_with(THUMB_PREFIX) {
                continue;
            }
            r.named.push(d.texture.clone());
            let Some(skate_ui::fe::retail_menu_movies::MenuImage::Retail { path, width, height }) = r.stage.image(&d.texture) else { continue };
            let mut bytes = crate::alloc::SCRATCH.lock().unwrap();
            match skate_data::vfs::read_into(&std::path::Path::new(crate::packs::ROOT).join(format!("{SET}/{path}")), &mut bytes) {
                Ok(()) if bytes.len() == width as usize * height as usize * 4 => apt.add_texture(&d.texture, width, height, &bytes),
                Ok(()) => println!("FE_MENUS image {}: {} bytes, the set says {width} x {height}", d.texture, bytes.len()),
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
/// The texture keys of Slappy's thumbnails (`CAC_GetThumbnailFileName` names through `ImageRenderer_Load`).
const THUMB_PREFIX: &str = skate_ui::fe::menu_stage::CAS_THUMB_PREFIX;
/// Thumbnails read and uploaded in one frame at most: a page fills in over three frames instead of one long one.
const THUMBS_PER_FRAME: usize = 3;

/// A retail prompt's button as the replay screens name their glyphs: the face buttons as they are, the others with
/// `_hud` (a stick or the D-pad without a direction suffix: the stick, the D-pad's arrows).
fn glyph_name(retail: &str) -> String {
    // The face buttons, Back and Start draw the same with or without `_hud` (the dropper's `Back_hud`, the language
    // strings' inline `<B:A_hud>`).
    let plain = retail.strip_suffix("_hud").unwrap_or(retail);
    if matches!(plain, "A" | "B" | "X" | "Y" | "Back" | "Start") {
        return plain.to_owned();
    }
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
