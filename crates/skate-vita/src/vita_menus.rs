//! R2c: the Vita's pause menu on the retail front end. Retail's free skate menu (mode `FreePlay`) keeps its four
//! tabs and their retail rows (the ones the Vita cannot run yet, such as Resume Career, stay greyed; online and music
//! leave it); the Vita's own rows join them: controls and stick calibration in Learn, quit at the end of
//! Options, and the Replay Editor lists the instant replay and the saved replays. The parks, districts and Skate
//! School are reached through the Challenge Map (the Vita's main menu keeps its own world lists). Game Settings keeps
//! retail's screens (Difficulty and the effects volume are the Vita's; the rows with no Vita equivalent stay greyed,
//! as the registry leaves rows without a binding) and gains three screens for the Vita's own settings: its controls,
//! the living world and the system.
use skate_core::menu_values::{number, ValueRule};
use skate_core::menus::SettingValue;
use skate_core::menus::{
    EntryRef, Handler, Icon, Item, Link, MenuContext, MenuRegistry, Owner, Setting, Tab, ValueHook, Widget,
};
use skate_ui::fe::crossbar_host::{sub_option_facts, SubOptionRow};
use skate_ui::fe::menu_stage::Customize;

use crate::settings::{Difficulty, Settings, PEDESTRIANS, TRAFFIC};

/// The registry owner of everything this module adds.
const OWNER: &str = "vita";
/// The free skate mode (`RetailMenus` context): the pause menu.
const MODE: &str = "FreePlay";
/// The Vita's main menu: a mode of the same crossbar.
const MAIN_MODE: &str = "VitaMain";

/// Sub-option lists of the Vita's items (after retail's kinds).
pub const PARKS_KIND: usize = 40;
pub const DISTRICTS_KIND: usize = 41;
pub const TRAINING_KIND: usize = 42;
pub const REPLAY_KIND: usize = 43;
/// The crossbar's sub-option column has six rows (`crossbar_host::MAX_SUB_OPTIONS`).
const COLUMN: usize = 6;

const SKATE: &str = "VitaSkate";
const RESPAWN: &str = "VitaRespawn";
const PARKS: &str = "VitaParks";
const DISTRICTS: &str = "VitaDistricts";
const TRAINING: &str = "VitaTraining";
const MAIN_MENU: &str = "VitaMainMenu";
const QUIT: &str = "VitaQuit";
const CONTROLS: &str = "VitaControls";
const CALIBRATION: &str = "VitaCalibration";
const REPLAY: &str = "ReplayEditor";
const ON_SCREEN_HELP: &str = "OnScreenHelp";
/// Retail's items: the stage gives them their engine handlers when their data and movies are there (greyed
/// otherwise).
const TRICK_GUIDE: &str = "TrickGuide";
const CHALLENGE_MAP: &str = "ChallengeMap";
const GAME_SETTINGS: &str = "GameSettings";
const EXTRAS: &str = "Extras";
const SKATE_PROFILE: &str = "SkateProfileOffline";
/// Retail's SinglePlayer rows the Vita cannot run yet: listed as retail has them, greyed (no handler).
const RESUME_CAREER: &str = "ResumeCareer";
const FREE_PLAY_OPTIONS: &str = "SoloFreeskateOptions";
const SKATE_WITH: &str = "SkateWith";

const DIFFICULTY: &str = "ID_GAMESETTINGS_DIFFICULTY_OPTIONS";
const SFX_VOLUME: &str = "ID_GAMESETTINGS_SFXVOLUME";
/// Retail's Minimap row (settings +158, default on): the HUD minimap (retail_hud.rs).
const MINIMAP: &str = "ID_GAMESETTINGS_MINIMAP_TOGGLE";
const REAR_TOUCH: &str = "VITA_REAR_TOUCH";
const DPAD_BUMPERS: &str = "VITA_DPAD_BUMPERS";
const FRONT_TOUCH: &str = "VITA_FRONT_TOUCH";
const TRAFFIC_ROW: &str = "VITA_TRAFFIC";
const PEDESTRIANS_ROW: &str = "VITA_PEDESTRIANS";
const AI_SKATERS: &str = "VITA_AI_SKATERS";
const PHYSICS: &str = "VITA_PHYSICS";
const AUDIO: &str = "VITA_AUDIO";
const DEBUG_HUD: &str = "VITA_DEBUG_HUD";
const MENU_STYLE: &str = "VITA_MENU_STYLE";

/// The worlds the menu offers: (index into `packs::WORLDS`, label), per list.
#[derive(Clone, Debug, Default)]
pub struct Worlds {
    pub parks: Vec<(usize, &'static str)>,
    pub districts: Vec<(usize, &'static str)>,
    pub training: Vec<(usize, &'static str)>,
}

/// What a choice in the menu asks of the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Main menu: skate the selected world.
    Skate,
    Respawn,
    /// Load and skate a world (index into `packs::WORLDS`).
    World(usize),
    MainMenu,
    Quit,
    Controls,
    Calibration,
    /// The instant replay (0) or the k-th saved replay.
    Replay(usize),
    TrickAnalyzer,
    ManualMeter,
}

fn row(labels: &[&str], helpers: &[&str]) -> SubOptionRow {
    SubOptionRow { labels: labels.iter().map(|l| (*l).to_owned()).collect(), helpers: helpers.iter().map(|h| (*h).to_owned()).collect() }
}

fn world_row(list: &[(usize, &str)], help: &str) -> SubOptionRow {
    let labels: Vec<&str> = list.iter().take(COLUMN).map(|(_, l)| *l).collect();
    row(&labels, &vec![help; labels.len()])
}

/// The menu structure: items, tabs, sub-option lists and the Vita's settings screens (before the registry).
pub fn customize(c: Customize, worlds: &Worlds) {
    let item = |id: &str, label: &str, icon: &str, help: &str, kind: i32| Item {
        id: id.into(),
        label: label.into(),
        icon: Icon::Retail(icon.into()),
        help: help.into(),
        sub_option_kind: kind,
    };
    c.data.items.extend([
        item(SKATE, "Skate", "freeskate", "Start skating.", -1),
        item(RESPAWN, "Respawn", "restart", "Back to this world's spawn point.", -1),
        item(PARKS, "Parks", "skatepark", "Load a skate park.", PARKS_KIND as i32),
        item(DISTRICTS, "Districts", "skatepark", "A campaign district, loaded in pieces while you skate.", DISTRICTS_KIND as i32),
        item(TRAINING, "Training", "skatepark", "Skate School, or the test course.", TRAINING_KIND as i32),
        item(MAIN_MENU, "Main menu", "quitsession", "Back to the main menu.", -1),
        item(QUIT, "Quit", "quit", "Leave the game.", -1),
        item(CONTROLS, "Controls", "trickguide", "Which button does what on the Vita.", -1),
        item(CALIBRATION, "Stick calibration", "onscreenhelp", "Check that both sticks reach every direction.", -1),
    ]);
    if let Some(replay) = c.data.items.iter_mut().find(|i| i.id == REPLAY) {
        replay.sub_option_kind = REPLAY_KIND as i32;
    }
    // Retail's tabs, with the Vita's rows (the others leave this mode).
    if let Some(mode) = c.data.modes.iter_mut().find(|m| m.key == MODE) {
        let rows = |category: &str| -> Vec<String> {
            match category {
                // Retail's rows (the parks, districts and Skate School are Challenge Map locations; the session marker
                // stands in for Respawn), quit at the end of Options (user 2026-10-23; the start-up menu is not reached
                // from here: the Challenge Map has every world).
                "SinglePlayer" => vec![CHALLENGE_MAP, RESUME_CAREER, FREE_PLAY_OPTIONS, SKATE_WITH, SKATE_PROFILE],
                "Create" => vec![REPLAY],
                "Learn" => vec![TRICK_GUIDE, ON_SCREEN_HELP, CONTROLS, CALIBRATION],
                "Options" => vec![GAME_SETTINGS, EXTRAS, QUIT],
                _ => vec![],
            }
            .into_iter()
            .map(String::from)
            .collect()
        };
        mode.tabs = mode.tabs.iter().map(|t| Tab { category: t.category.clone(), items: rows(&t.category) }).filter(|t| !t.items.is_empty()).collect();
        // The main menu: the same tabs without what needs a session (respawn, the replay, the main menu itself).
        let main_rows = |category: &str| -> Vec<String> {
            match category {
                "SinglePlayer" => vec![SKATE, CHALLENGE_MAP, PARKS, DISTRICTS, TRAINING, QUIT],
                "Learn" => vec![TRICK_GUIDE, CONTROLS, CALIBRATION],
                "Options" => vec![GAME_SETTINGS],
                _ => vec![],
            }
            .into_iter()
            .map(String::from)
            .collect()
        };
        let main = skate_core::menus::Mode {
            key: MAIN_MODE.into(),
            title: "Skate Vita".into(),
            slots: mode.slots,
            tabs: mode.tabs.iter().map(|t| Tab { category: t.category.clone(), items: main_rows(&t.category) }).filter(|t| !t.items.is_empty()).collect(),
        };
        c.data.modes.push(main);
    }
    let sub = &mut *c.sub_options;
    if sub.len() <= REPLAY_KIND {
        sub.resize_with(REPLAY_KIND + 1, || row(&[], &[]));
    }
    sub[PARKS_KIND] = world_row(&worlds.parks, "Loads the park and starts skating.");
    sub[DISTRICTS_KIND] = world_row(&worlds.districts, "Loads the district and starts skating.");
    sub[TRAINING_KIND] = world_row(&worlds.training, "Loads it and starts skating.");
    sub[REPLAY_KIND] = row(&["Instant replay"], &["The last minute of skating."]);
    // Game Settings: three screens for the Vita's own settings, linked from the main list (screens 0 and 1).
    let setting = |id: &str, label: &str, widget: Widget| Setting { id: id.into(), label: label.into(), widget, link: None };
    let screens: [(&str, &str, Vec<Setting>); 3] = [
        (
            "VITA_CONTROL_SETTINGS",
            "Vita controls",
            vec![
                setting(REAR_TOUCH, "Rear touch as LB / RB", Widget::Selector),
                setting(DPAD_BUMPERS, "D-pad left / right as LB / RB", Widget::Selector),
                setting(FRONT_TOUCH, "Front touch as L3 / R3", Widget::Selector),
            ],
        ),
        (
            "VITA_WORLD_SETTINGS",
            "World",
            vec![
                setting(TRAFFIC_ROW, "Traffic", Widget::Selector),
                setting(PEDESTRIANS_ROW, "Pedestrians", Widget::Selector),
                setting(AI_SKATERS, "AI skaters", Widget::Selector),
                setting(PHYSICS, "Physics", Widget::Selector),
            ],
        ),
        (
            "VITA_SYSTEM_SETTINGS",
            "Vita system",
            vec![
                setting(AUDIO, "Board sounds", Widget::Selector),
                setting(DEBUG_HUD, "Debug overlay", Widget::Selector),
                setting(MENU_STYLE, "Menu style", Widget::Selector),
            ],
        ),
    ];
    for (link, title, rows) in screens {
        let screen = c.data.screens.len();
        c.data.screens.push(rows.iter().map(|r| r.id.clone()).collect());
        c.data.settings.extend(rows);
        c.data.settings.push(Setting { id: link.into(), label: title.into(), widget: Widget::Option, link: Some(Link::Screen(screen)) });
        for main in c.data.screens.iter_mut().take(2) {
            main.push(link.into());
        }
        if c.settings_titles.len() < screen {
            let filler = c.settings_titles.last().cloned().unwrap_or_default();
            c.settings_titles.resize(screen, filler);
        }
        c.settings_titles.push(title.into());
    }
}

fn owner() -> Owner {
    Owner::Engine(OWNER.into())
}

fn toggle() -> ValueRule {
    ValueRule::Toggle { labels: ["ID_COMMON_OFF".into(), "ID_COMMON_ON".into()] }
}

fn cycle(labels: &[&str]) -> ValueRule {
    ValueRule::Cycle { count: labels.len() as i64, labels: labels.iter().map(|l| (*l).to_owned()).collect() }
}

/// The registry side: handlers for the Vita's items, rules and values for its settings rows.
pub fn register(registry: &mut MenuRegistry, settings: &Settings) {
    for id in [SKATE, RESPAWN, PARKS, DISTRICTS, TRAINING, MAIN_MENU, QUIT, CONTROLS, CALIBRATION, REPLAY, ON_SCREEN_HELP] {
        if registry.data().item(id).is_some() {
            registry.register_handler(EntryRef::item(id), Handler { owner: owner(), select: true, highlight: false, callback: None });
        }
    }
    let traffic: Vec<&str> = TRAFFIC.iter().map(|t| t.0).collect();
    let pedestrians: Vec<&str> = PEDESTRIANS.iter().map(|t| t.0).collect();
    let difficulty: Vec<String> = ["EASY", "NORMAL", "HARDCORE"].iter().map(|l| format!("ID_GAMESETTINGS_DIFFICULTY_{l}")).collect();
    let rules: [(&str, ValueRule); 13] = [
        (DIFFICULTY, cycle(&difficulty.iter().map(String::as_str).collect::<Vec<_>>())),
        (SFX_VOLUME, ValueRule::retail_slider()),
        (MINIMAP, toggle()),
        (REAR_TOUCH, toggle()),
        (DPAD_BUMPERS, toggle()),
        (FRONT_TOUCH, toggle()),
        (TRAFFIC_ROW, cycle(&traffic)),
        (PEDESTRIANS_ROW, cycle(&pedestrians)),
        (AI_SKATERS, toggle()),
        (PHYSICS, cycle(&["Fast", "Retail"])),
        (AUDIO, cycle(&["Retail", "Synth"])),
        (DEBUG_HUD, toggle()),
        (MENU_STYLE, cycle(&["Retail", "Plain"])),
    ];
    for (id, rule) in rules {
        if registry.data().setting(id).is_some() {
            registry.set_rule(EntryRef::setting(id), owner(), rule);
            registry.bind_value(EntryRef::setting(id), owner(), ValueHook::Stored(SettingValue::Int(0)));
        }
    }
    write_settings(registry, settings);
}

/// The settings rows' values from the game's settings (on open: something else may have changed them).
pub fn write_settings(registry: &mut MenuRegistry, s: &Settings) {
    let c = &s.controls;
    let int = |v: u8| SettingValue::Int(i64::from(v));
    let flag = |b: bool| SettingValue::Int(i64::from(b));
    let difficulty = match s.difficulty {
        Difficulty::Easy => 0,
        Difficulty::Normal => 1,
        Difficulty::Hardcore => 2,
    };
    let values = [
        (DIFFICULTY, int(difficulty)),
        (SFX_VOLUME, SettingValue::Float(f64::from(s.volume.min(10)) / 10.0)),
        (MINIMAP, flag(s.minimap)),
        (REAR_TOUCH, flag(c.rear_touch_bumpers)),
        (DPAD_BUMPERS, flag(c.dpad_bumpers)),
        (FRONT_TOUCH, flag(c.front_touch_thumbs)),
        (TRAFFIC_ROW, int(s.traffic)),
        (PEDESTRIANS_ROW, int(s.pedestrians)),
        (AI_SKATERS, flag(s.ai_skaters)),
        (PHYSICS, flag(!s.fast_physics)),
        (AUDIO, flag(!s.retail_audio)),
        (DEBUG_HUD, flag(s.debug_hud)),
        (MENU_STYLE, flag(!s.retail_menus)),
    ];
    for (id, value) in values {
        if registry.data().setting(id).is_some() {
            registry.set_owner_value(&EntryRef::setting(id), &owner(), value);
        }
    }
}

/// A settings row the player changed: the game's settings follow it. True when something changed.
pub fn read_setting(registry: &MenuRegistry, id: &str, s: &mut Settings) -> bool {
    let Some(value) = registry.value(&EntryRef::setting(id)) else { return false };
    let v = number(&value);
    let index = |count: usize| (v.max(0.0) as usize).min(count - 1) as u8;
    let on = v != 0.0;
    let before = s.clone();
    match id {
        DIFFICULTY => s.difficulty = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hardcore][index(3) as usize],
        SFX_VOLUME => s.volume = (v * 10.0).round().clamp(0.0, 10.0) as u8,
        MINIMAP => s.minimap = on,
        REAR_TOUCH => s.controls.rear_touch_bumpers = on,
        DPAD_BUMPERS => s.controls.dpad_bumpers = on,
        FRONT_TOUCH => s.controls.front_touch_thumbs = on,
        TRAFFIC_ROW => s.traffic = index(TRAFFIC.len()),
        PEDESTRIANS_ROW => s.pedestrians = index(PEDESTRIANS.len()),
        AI_SKATERS => s.ai_skaters = on,
        PHYSICS => s.fast_physics = !on,
        AUDIO => s.retail_audio = !on,
        DEBUG_HUD => s.debug_hud = on,
        MENU_STYLE => s.retail_menus = !on,
        _ => return false,
    }
    *s != before
}

/// The crossbar's mode: the main menu (it stays: Back and Start do not close it) or the pause menu.
pub fn set_main_menu(context: &mut MenuContext, main: bool) {
    context.mode = if main { MAIN_MODE } else { MODE }.into();
}

/// The facts the crossbar reads: the on-screen help rows offer "off" for what is on.
pub fn set_facts(context: &mut MenuContext, s: &Settings) {
    context.facts.insert(sub_option_facts::TRICK_ANALYZER.into(), f64::from(u8::from(s.trick_analyzer)));
    context.facts.insert(sub_option_facts::MANUAL_METER.into(), f64::from(u8::from(s.manual_meter)));
}

/// The Replay Editor's list: the instant replay, then the saved replays (as many as the column holds).
pub fn replay_row(choices: &[String]) -> SubOptionRow {
    let labels: Vec<&str> = choices.iter().take(COLUMN).map(String::as_str).collect();
    row(&labels, &vec!["Watch and edit it."; labels.len()])
}

/// What an item (or one of its sub-option rows) asks of the game.
pub fn choice(item: &str, sub: Option<usize>, worlds: &Worlds) -> Option<Choice> {
    let world = |list: &[(usize, &str)]| sub.and_then(|k| list.get(k)).map(|(w, _)| Choice::World(*w));
    match (item, sub) {
        (SKATE, _) => Some(Choice::Skate),
        (RESPAWN, _) => Some(Choice::Respawn),
        (PARKS, _) => world(&worlds.parks),
        (DISTRICTS, _) => world(&worlds.districts),
        (TRAINING, _) => world(&worlds.training),
        (MAIN_MENU, _) => Some(Choice::MainMenu),
        (QUIT, _) => Some(Choice::Quit),
        (CONTROLS, _) => Some(Choice::Controls),
        (CALIBRATION, _) => Some(Choice::Calibration),
        (REPLAY, Some(k)) => Some(Choice::Replay(k)),
        (ON_SCREEN_HELP, Some(0)) => Some(Choice::TrickAnalyzer),
        (ON_SCREEN_HELP, Some(1)) => Some(Choice::ManualMeter),
        _ => None,
    }
}

/// The settings rows the registry knows (tests).
#[cfg(test)]
fn rows_of(data: &skate_core::menus::MenuData, screen: usize) -> Vec<String> {
    data.screens.get(screen).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::menu_values::Direction;
    use skate_core::menus::{MenuAction, MenuData, Mode};

    fn worlds() -> Worlds {
        Worlds {
            parks: vec![(1, "Start Park"), (2, "Mega Park")],
            districts: vec![(8, "Industrial")],
            training: vec![(7, "Skate School"), (0, "Course")],
        }
    }

    fn data() -> MenuData {
        let tab = |c: &str, items: &[&str]| Tab { category: c.into(), items: items.iter().map(|i| (*i).to_owned()).collect() };
        let item = |id: &str| Item { id: id.into(), label: id.into(), icon: Icon::Retail("x".into()), help: String::new(), sub_option_kind: -1 };
        MenuData {
            items: [REPLAY, ON_SCREEN_HELP, GAME_SETTINGS, "ChallengeMap", "TrickGuide"].map(item).to_vec(),
            settings: vec![Setting { id: DIFFICULTY.into(), label: "d".into(), widget: Widget::Selector, link: None }],
            screens: vec![vec![], vec![], vec![DIFFICULTY.into()]],
            modes: vec![Mode {
                key: MODE.into(),
                title: "t".into(),
                slots: 8,
                tabs: vec![
                    tab("SinglePlayer", &["ChallengeMap"]),
                    tab("Create", &[REPLAY]),
                    tab("Learn", &["TrickGuide", ON_SCREEN_HELP]),
                    tab("Options", &[GAME_SETTINGS]),
                ],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn the_vita_layout_and_settings_round_trip() {
        let mut data = data();
        let (mut sub, mut titles) = (Vec::new(), vec!["T".to_owned(); 3]);
        customize(Customize { data: &mut data, sub_options: &mut sub, settings_titles: &mut titles }, &worlds());
        let mode = data.mode(MODE).unwrap();
        assert_eq!(mode.tabs[0].items, [CHALLENGE_MAP, RESUME_CAREER, FREE_PLAY_OPTIONS, SKATE_WITH, SKATE_PROFILE]);
        assert_eq!(mode.tabs[3].items, [GAME_SETTINGS, EXTRAS, QUIT]);
        assert_eq!(mode.tabs[2].items, [TRICK_GUIDE, ON_SCREEN_HELP, CONTROLS, CALIBRATION]);
        assert_eq!(sub[TRAINING_KIND].labels, ["Skate School", "Course"]);
        assert_eq!(titles.len(), 6, "three Vita screens after retail's");
        assert_eq!(rows_of(&data, 3), [REAR_TOUCH, DPAD_BUMPERS, FRONT_TOUCH]);
        assert!(rows_of(&data, 0).contains(&"VITA_WORLD_SETTINGS".to_owned()));
        let mut registry = MenuRegistry::new(data);
        let mut settings = Settings::default();
        register(&mut registry, &settings);
        let context = MenuContext::new(MODE);
        assert!(registry.enabled(&EntryRef::item(PARKS), &context));
        assert!(registry.enabled(&EntryRef::setting(REAR_TOUCH), &context));
        // Right on Traffic steps the stored value; the settings follow.
        let before = settings.traffic;
        registry.dispatch(MenuAction::Step { entry: EntryRef::setting(TRAFFIC_ROW), direction: Direction::Right }, &context);
        assert!(read_setting(&registry, TRAFFIC_ROW, &mut settings));
        assert_eq!(settings.traffic, (before + 1) % TRAFFIC.len() as u8);
        registry.dispatch(MenuAction::Step { entry: EntryRef::setting(REAR_TOUCH), direction: Direction::Left }, &context);
        assert!(read_setting(&registry, REAR_TOUCH, &mut settings));
        assert_eq!(settings.controls.rear_touch_bumpers, !Settings::default().controls.rear_touch_bumpers);
        assert_eq!(choice(TRAINING, Some(1), &worlds()), Some(Choice::World(0)));
        assert_eq!(choice(ON_SCREEN_HELP, Some(1), &worlds()), Some(Choice::ManualMeter));
    }
}

/// The Vita's menu on the retail data and movies: Parks' second row loads that park; Game Settings lists the Vita
/// screens and Rear touch steps. Data-gated: `SKATE3_MENU_MOVIES=<set>`, `SKATE3_MAIN_CHECKOUT=<install>`.
#[cfg(test)]
mod retail_tests {
    use super::*;
    use skate_ui::fe::menu_stage::{MenuStage, APT_NEXT, APT_PREVIOUS, APT_RIGHT, APT_START, PAUSE_ROOTS};
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;

    #[test]
    fn the_vita_pause_menu_on_the_retail_movies() {
        let (Some(set), Some(install)) = (std::env::var_os("SKATE3_MENU_MOVIES"), std::env::var_os("SKATE3_MAIN_CHECKOUT")) else {
            eprintln!("skipped: SKATE3_MENU_MOVIES / SKATE3_MAIN_CHECKOUT not set");
            return;
        };
        let set = std::path::PathBuf::from(set);
        let assets = std::path::PathBuf::from(install).join("assets");
        let read_set = |p: &str| std::fs::read(set.join(p)).map_err(|e| e.to_string());
        // The Trick Guide's data is baked into ui.pak (vita-bake); here it is built the same way.
        let read = |p: &str| -> Result<Vec<u8>, String> {
            if p == skate_ui::fe::menu_stage::TRICK_GUIDE_DATA {
                let tables = skate_data::menu_tables::MenuTables::from_json(&std::fs::read_to_string(assets.join("private/menu-tables.json")).map_err(|e| e.to_string())?)?;
                let guide = skate_data::trick_guide::load(&skate_data::collections::Collections::load(&assets)?, &tables.trick_guide);
                return serde_json::to_vec(&skate_data::trick_guide::to_json(&guide)).map_err(|e| e.to_string());
            }
            std::fs::read(assets.join(p)).map_err(|e| e.to_string())
        };
        let worlds = Worlds { parks: vec![(1, "Start Park"), (2, "Mega Park")], districts: vec![(8, "Industrial")], training: vec![(7, "Skate School")] };
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot_with(movies, &read, &mut |c| customize(c, &worlds)).unwrap();
        let mut settings = Settings::default();
        register(&mut ms.menus.registry, &settings);
        let log = std::cell::RefCell::new(Vec::<String>::new());
        // A D-pad press fires AptDown and AptDDown (17) in one frame, as the pad does.
        let mut run = |ms: &mut MenuStage, key: Option<u32>, frames: usize| {
            let mut out = skate_ui::fe::menu_stage::Frame::default();
            for f in 0..frames {
                let keys = match key.filter(|_| f == 0) {
                    Some(13) => vec![(13, 0), (17, 0)],
                    Some(k) => vec![(k, 0)],
                    None => Vec::new(),
                };
                let mut frame = ms.update_keys(keys, 1.0 / 60.0);
                log.borrow_mut().append(&mut frame.log.clone());
                out.sub_options.append(&mut frame.sub_options);
                out.items.append(&mut frame.items);
                out.settings.append(&mut frame.settings);
            }
            assert!(!ms.failed, "{log:?}");
            out
        };
        run(&mut ms, Some(APT_START), 90);
        assert!(ms.open());
        // (The parks are the main menu's now: `a_columns_highlight_moves_with_down` picks one there.)
        // Options (three tabs right): Game Settings; the main list's last three rows are the Vita's screens.
        for _ in 0..3 {
            run(&mut ms, Some(APT_RIGHT), 15);
        }
        run(&mut ms, Some(APT_NEXT), 120);
        // Enabled rows: Difficulty, Audio, Video, then the Vita's (Controls, Online and Skatefeed are greyed: nothing
        // binds their rows); the movie skips greyed rows.
        for _ in 0..3 {
            run(&mut ms, Some(13), 12);
        }
        run(&mut ms, Some(APT_NEXT), 60);
        let out = run(&mut ms, Some(APT_RIGHT), 30);
        let changed: Vec<&String> = out.settings.iter().map(|(id, _)| id).collect();
        assert_eq!(changed, [REAR_TOUCH], "{log:?}");
        assert!(read_setting(&ms.menus.registry, REAR_TOUCH, &mut settings));
        assert_ne!(settings.controls.rear_touch_bumpers, Settings::default().controls.rear_touch_bumpers);
        // Back to the pause menu (it reopens on its first tab), Learn (two tabs right): the Trick Guide opens (after
        // its fade), a category opens its list, Back goes up and Back closes it (fading out).
        run(&mut ms, Some(APT_PREVIOUS), 60);
        run(&mut ms, Some(APT_PREVIOUS), 180);
        assert!(ms.open(), "{log:?}");
        run(&mut ms, Some(APT_RIGHT), 20);
        run(&mut ms, Some(APT_RIGHT), 20);
        run(&mut ms, Some(APT_NEXT), 240);
        assert!(log.borrow().iter().any(|l| l.starts_with("open Trick Guide")), "{log:?}");
        assert!(ms.draw().len() > 20, "the Trick Guide draws");
        run(&mut ms, Some(APT_NEXT), 60);
        run(&mut ms, Some(APT_PREVIOUS), 60);
        run(&mut ms, Some(APT_PREVIOUS), 120);
        assert!(log.borrow().iter().any(|l| l.starts_with("close Trick Guide")), "{log:?}");
        assert!(!ms.failed);
    }
}

/// The crossbar's sub-option columns highlight their row (the picker greys every row the focus screen does not call
/// enabled) and the highlight moves with Down: Parks, the Replay Editor's list, On-screen help. Data-gated like
/// `retail_tests`.
#[cfg(test)]
mod column_tests {
    use super::*;
    use skate_ui::fe::menu_stage::{MenuStage, APT_NEXT, APT_RIGHT, APT_START, PAUSE_ROOTS};
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;

    fn summary(ms: &mut MenuStage) -> Vec<String> {
        ms.draw().iter().map(|d| format!("{:?} {:?} {}", d.vertices.first().map(|v| v.position), d.multiply, d.texture)).collect()
    }

    #[test]
    fn a_columns_highlight_moves_with_down() {
        let (Some(set), Some(install)) = (std::env::var_os("SKATE3_MENU_MOVIES"), std::env::var_os("SKATE3_MAIN_CHECKOUT")) else {
            eprintln!("skipped: SKATE3_MENU_MOVIES / SKATE3_MAIN_CHECKOUT not set");
            return;
        };
        let set = std::path::PathBuf::from(set);
        let assets = std::path::PathBuf::from(install).join("assets");
        let read_set = |p: &str| std::fs::read(set.join(p)).map_err(|e| e.to_string());
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| e.to_string());
        let worlds = Worlds { parks: vec![(1, "Start Park"), (2, "Mega Park")], districts: vec![(8, "Industrial")], training: vec![(7, "Skate School")] };
        for target in [PARKS, REPLAY, ON_SCREEN_HELP] {
            let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
            let mut ms = MenuStage::boot_with(movies, &read, &mut |c| customize(c, &worlds)).unwrap();
            register(&mut ms.menus.registry, &Settings::default());
            ms.set_sub_options(REPLAY_KIND, replay_row(&["Instant replay".to_owned(), "Saved one".to_owned()]));
            // The world lists are the main menu's (the pause menu reaches the worlds through the Challenge Map).
            set_main_menu(&mut ms.menus.context, target == PARKS);
            let run = |ms: &mut MenuStage, keys: Vec<(u32, usize)>, frames: usize| {
                ms.update_keys(keys, 1.0 / 60.0);
                for _ in 1..frames {
                    ms.update_keys(Vec::new(), 1.0 / 60.0);
                }
            };
            run(&mut ms, vec![(APT_START, 0)], 90);
            let view = skate_ui::fe::crossbar_host::FrontEnd::new(&ms.menus.registry, &ms.menus.context, &Default::default()).view;
            let (tab, row) = (0..8).flat_map(|t| (0..16).map(move |r| (t, r))).find(|&(t, r)| view.item(t, r) == Some(target)).expect(target);
            for _ in 0..tab {
                run(&mut ms, vec![(APT_RIGHT, 0)], 20);
            }
            // The first enabled row is where the tab opens (rows before it may be hidden in this mode).
            while ms.crossbar_option() < row as i32 {
                run(&mut ms, vec![(13, 0), (17, 0)], 12);
            }
            run(&mut ms, vec![(APT_NEXT, 0)], 30);
            let open = summary(&mut ms);
            run(&mut ms, vec![(13, 0), (17, 0)], 30);
            assert_ne!(summary(&mut ms), open, "{target}: the column's highlight moves");
            if target == PARKS {
                // The second row is Mega Park: A picks it.
                let frame = ms.update_keys(vec![(APT_NEXT, 0)], 1.0 / 60.0);
                let picked: Vec<Option<Choice>> = frame.sub_options.iter().map(|(item, sub)| choice(item, Some(*sub), &worlds)).collect();
                assert_eq!(picked, [Some(Choice::World(2))], "{:?}", frame.sub_options);
            }
        }
    }
}

/// The Challenge Map on the retail data and movies: it opens from Single Player, draws a world map, and A down the
/// tree picks a location whose teleport the stage gives back. Data-gated like `retail_tests`.
#[cfg(test)]
mod map_tests {
    use super::*;
    use skate_ui::fe::menu_stage::{MenuStage, APT_NEXT, APT_START, PAUSE_ROOTS};
    use skate_ui::fe::retail_menu_movies::RetailMenuMovies;

    #[test]
    fn the_challenge_map_picks_a_location() {
        let (Some(set), Some(install)) = (std::env::var_os("SKATE3_MENU_MOVIES"), std::env::var_os("SKATE3_MAIN_CHECKOUT")) else {
            eprintln!("skipped: SKATE3_MENU_MOVIES / SKATE3_MAIN_CHECKOUT not set");
            return;
        };
        let set = std::path::PathBuf::from(set);
        let assets = std::path::PathBuf::from(install).join("assets");
        let read_set = |p: &str| std::fs::read(set.join(p)).map_err(|e| e.to_string());
        // The map's data is baked into ui.pak (vita-bake); here it is built the same way.
        let read = |p: &str| -> Result<Vec<u8>, String> {
            if p == skate_ui::fe::menu_stage::CHALLENGE_MAP_DATA {
                let collections = skate_data::collections::Collections::load(&assets)?;
                let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(set.join("manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                let world_maps = serde_json::from_value(manifest["world_maps"].clone()).unwrap_or_default();
                let teleports: serde_json::Value = serde_json::from_slice(&std::fs::read(assets.join("private/teleports.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                return serde_json::to_vec(&skate_data::challenge_map::bake(&collections, &world_maps, &teleports, &skate_data::challenge_map::LocationUnlocks::load(&assets)).to_json()).map_err(|e| e.to_string());
            }
            std::fs::read(assets.join(p)).map_err(|e| e.to_string())
        };
        let worlds = Worlds::default();
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot_with(movies, &read, &mut |c| customize(c, &worlds)).unwrap();
        register(&mut ms.menus.registry, &Settings::default());
        let travel = ms.register_teleports(&|map| ["DownTown", "University", "Industrial", "StartPark"].contains(&map));
        assert!(travel > 0);
        let log = std::cell::RefCell::new(Vec::<String>::new());
        let teleports = std::cell::RefCell::new(Vec::<String>::new());
        let mut run = |ms: &mut MenuStage, key: Option<u32>, frames: usize| {
            for f in 0..frames {
                let keys = match key.filter(|_| f == 0) {
                    Some(13) => vec![(13, 0), (17, 0)],
                    Some(k) => vec![(k, 0)],
                    None => Vec::new(),
                };
                let mut frame = ms.update_keys(keys, 1.0 / 60.0);
                log.borrow_mut().append(&mut frame.log);
                teleports.borrow_mut().append(&mut frame.teleports);
            }
            assert!(!ms.failed, "{log:?}");
        };
        run(&mut ms, Some(APT_START), 90);
        // Single Player's first row (where the menu opens): the Challenge Map.
        let first = ms.menus.registry.tab_rows("SinglePlayer", &ms.menus.context);
        assert_eq!(first.first().map(|r| r.id.as_str()), Some(CHALLENGE_MAP));
        assert!(first[0].enabled, "the map item has its handler");
        run(&mut ms, Some(APT_NEXT), 150);
        assert!(log.borrow().iter().any(|l| l.starts_with("open Challenge Map")), "{log:?}");
        let draws = ms.draw();
        let images: Vec<&str> = draws.iter().map(|d| d.texture.as_str()).filter(|t| t.starts_with("image/")).collect();
        eprintln!("MAP draws {} ({} named images: {:?})", draws.len(), images.len(), &images[..images.len().min(5)]);
        // A down the tree until a location is picked.
        for _ in 0..6 {
            if !teleports.borrow().is_empty() {
                break;
            }
            run(&mut ms, Some(APT_NEXT), 60);
        }
        eprintln!("MAP picked {teleports:?}; log {:?}", log.borrow().iter().filter(|l| l.contains("challenge map")).collect::<Vec<_>>());
        let key = teleports.borrow().first().cloned().expect("a location picked");
        let (map, matrix) = ms.teleport(&key).expect("its teleport");
        assert!(matrix.iter().flatten().all(|v| v.is_finite()) && !map.is_empty());
    }
}
