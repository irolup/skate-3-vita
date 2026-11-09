//! The retail front end's per-frame glue without Bevy (skate_vita R2b): skate-game `retail_menu_stage.rs` (PR #52
//! `fad5aae`) `setup`, `update` and the draw list of `render`, for the pause menu (FE 56, core_menu), Game Settings
//! (FE 45), the popups (FE 29) and Video Calibration (FE 51). The order of the work and every retail note are
//! upstream's; what this port leaves out:
//! - the Trick Guide (FE 14) and the Challenge Map (FE 17): later (their hosts are stand-ins here), so their pause
//!   items get no engine handler;
//! - mods (popups, events, guide layers), the PC overlay, the keyboard and the `SKATE3_RETAIL_MENUS_*` switches;
//! - the stage restart after an error: the Vita drops the whole front end and builds it again ([`MenuStage::failed`]);
//! - the backdrop blur pass and the textures: the caller draws [`MenuStage::draw`] and plays [`Frame::sounds`].
use crate::fe::apt_scene::{Draw, Fill, Vertex};
use crate::fe::apt_stage::Stage;
use crate::fe::apt_vm::Value;
use crate::fe::crossbar_host::{FrontEnd, SubOptionRow};
use crate::fe::retail_menu_movies::{MenuImage, RetailMenuMovies};
use crate::fe::retail_menus::{
    RetailMenus, CHALLENGE_MAP_SCREEN, EXTRAS_SCREEN, OFFLINE_PROFILE_SCREEN, PAUSE_SCREEN, SCREEN, SETTINGS_SCREEN, TABLES, TEXT_ENTRY_SCREEN, TEXT_VIEWER_SCREEN,
    TRICK_GUIDE_SCREEN, VIDEO_CALIBRATION_SCREEN, FREE_SKATE_SCREEN, SPAWN_SKATER_SCREEN, LOADING_SCREEN,
};
use skate_core::crossbar::{CrossbarAction, CrossbarRules, CrossbarState, NavEvent};
use skate_core::fe_fade::{FeFade, FeFadeParams};
use skate_core::fe_input::{fired, parse_apt_keys, AptKey, FeInput, RepeatRule};
use skate_core::fe_popup::{PopupKind, PopupList, PopupRequest};
use skate_core::fe_stack::{Effect, FeStack, HudMode, Layer, Request, ScreenInfo};
use skate_core::menu_values::Direction;
use skate_core::menus::{EntryRef, Handler, MenuAction, Outcome, Owner};
use skate_core::video_calibration::{CalibrationAction, CalibrationRules, VideoCalibration};
use skate_data::menu_tables::{FeScreen, MenuTables};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Reads a file by its path relative to the asset root (`private/menu-tables.json`, ...).
pub type Reader<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, String>;

/// skate_vita: the fe sound records' names as a JSON list (vita-bake, from the audio manifest).
pub const FE_SOUNDS: &str = "private/menu-sounds.json";
pub const AUDIO_MANIFEST: &str = "private/audio/audio_manifest.json";

const CROSSBAR: &str = "CrossBar";
/// Key index of AptStart (Apt key table record 0).
pub const APT_START: u32 = 0;
/// AptNext (A), AptPrevious (B), AptLeft, AptRight key indices.
pub const APT_NEXT: u32 = 2;
pub const APT_PREVIOUS: u32 = 3;
pub const APT_LEFT: u32 = 14;
pub const APT_RIGHT: u32 = 15;
/// The pause menu template's path under `source/screens/` (template table 0x8302F1A8, record 69).
const CROSSBAR_PATH: &str = "main/core_menu.swf";
/// The Game Settings template (record 90) and its path; FE screen 45's init opens it (sub_8260BF58).
const OPTIONS: &str = "Options";
const OPTIONS_PATH: &str = "options/options.swf";
/// The Video Calibration template (record 91) and its path; FE screen 51's init opens it (0x8260E868).
const VIDEO_CALIBRATION: &str = "VideoCalibration";
const VIDEO_CALIBRATION_PATH: &str = "options/videocal.swf";
/// The callback id of its "finished" popup (0x8260EA48).
const VIDEO_CALIBRATION_POPUP: &str = "video_calibration.finished";
/// skate_vita: the customisation's gender switch question (825FE3C0 item 0x38).
const CAC_GENDER_POPUP: &str = "cac.gender";
/// The texture keys of Slappy's thumbnails (`CAC_GetThumbnailFileName` names loaded through `ImageRenderer_Load`):
/// drawn although the menu set lacks them, the game supplies the pictures.
pub const CAS_THUMB_PREFIX: &str = "image/fe/cas/";
/// The customisation's exit question (82603290: `ID_CAC_EXIT_CAC_CONFIRMATION`, Yes / No; answered by 82604748).
const CAC_EXIT_POPUP: &str = "cac.exit";
/// The tattoo regions' change questions (825FE3C0 menus 4 and 5: `ID_CAC_UPPER_TATTOO_CHANGE_CONFIRMATION`, answered by
/// 82604618; `ID_CAC_LOWER_TATTOO_CHANGE_CONFIRMATION`, 82604658).
const CAC_TATTOO_UPPER_POPUP: &str = "cac.tattoo.upper";
const CAC_TATTOO_LOWER_POPUP: &str = "cac.tattoo.lower";
/// The settings row that pushes it (sub_8260D290 row 25).
const VIDEO_CALIBRATION_ROW: &str = "ID_GAMESETTINGS_VIDEO_CALIBRATION";
/// The fader bit screen changes hold (sub_825E6B30 / sub_825E6BF8 bit 0).
const FADE_SCREEN: u32 = 0;
/// The Trick Guide template (record 29) and its path; FE screen 14's init opens it (0x82618A40). The pause item that
/// opens it (crossbar case 0x8261BF78).
const TRICK_GUIDE: &str = "TrickTutorial";
const TRICK_GUIDE_PATH: &str = "tricks/trickguide.swf";
const TRICK_GUIDE_ITEM: &str = "TrickGuide";
/// Trick Guide state 4 (0x82619A20): updates before it releases the fade and takes input (S+336 > 10).
const TRICK_GUIDE_FADE_IN_TICKS: u32 = 11;
/// skate_vita: the Trick Guide's data (vita-bake builds it from the skater collections and the menu tables).
pub const TRICK_GUIDE_DATA: &str = "private/trick-guide.json";
/// The Challenge Map template (record 68) and its path; FE screen 17 (b244 / b253). The pause item that opens it.
const CHALLENGE_MAP: &str = "ChallengeMap";
const CHALLENGE_MAP_PATH: &str = "map/challengemap.swf";
const CHALLENGE_MAP_ITEM: &str = "ChallengeMap";
/// Its tab keys (handler 82646B28, b256): AptRShift2 / AptDRight / AptLStickRight next, AptLShift2 / AptDLeft /
/// AptLStickLeft previous (clamped, no wrap).
const MAP_NEXT_TAB: [u32; 3] = [7, 19, 27];
const MAP_PREVIOUS_TAB: [u32; 3] = [10, 18, 26];
/// Owner prefix of the engine handlers that teleport to a Challenge Map location (`teleport:<fe_locations key>`).
pub const TELEPORT: &str = "teleport:";
/// skate_vita: the Challenge Map's data (vita-bake: `skate_data::challenge_map::Baked`).
pub const CHALLENGE_MAP_DATA: &str = "private/challenge-map.json";
/// The instant replay screen and its help (FE 12 / FE 13; b391): template 72 Replay2 with help 73 ReplayHelp
/// (upstream also opens the Create pack's Replay3 when the pack is owned; not taken). Opened by the engine with the
/// replay, not by a menu push.
const REPLAY: &str = "Replay2";
const REPLAY_PATH: &str = "replay2/replay_editor2.swf";
const REPLAY_HELP: &str = "ReplayHelp";
const REPLAY_HELP_PATH: &str = "replay2/replay_help.swf";
/// The Object Dropper screen (docs/object-dropper-survey.md "The screen"): template 103 ObjectDropper, opened by the
/// engine with the editor (FE state 0x35 in a park, 0x37 in the open world), not by a menu push.
const OBJECT_DROPPER: &str = "ObjectDropper";
const OBJECT_DROPPER_PATH: &str = "objectdropper/objectdropper.swf";
/// Its Quick Menu ("Properties", FE state 0x36, template 104), opened over it while the editor's Quick Menu is open.
const OBJECT_QUICK_MENU: &str = "ObjectQuickMenu";
const OBJECT_QUICK_MENU_PATH: &str = "objectdropper/objectquickmenu.swf";
/// The shared browser on parks (skate.Park > My Parks; FE 0x3B, screen record 59): template CommonBrowser, pushed by the
/// game ([`MenuStage::open_browser`]).
const BROWSER: &str = "CommonBrowser";
const BROWSER_PATH: &str = "browser/browser.swf";
const BROWSER_SCREEN: &str = "f34a9fae51e2b903";
/// Park Setup (skate.Park > New Park; FE 0x3A, screen record 58): template ParkSetup, pushed by the game
/// ([`MenuStage::open_park_setup`]).
const PARK_SETUP: &str = "ParkSetup";
const PARK_SETUP_PATH: &str = "skatepark/parksetup.swf";
const PARK_SETUP_SCREEN: &str = "d2dfe72ee96887b1";
/// The character customisation (retail FE 15, template 92; `MenuStage::open_cac`).
const CAC: &str = "CreateASkater";
const CAC_PATH: &str = "cas/createskater.swf";
const CAC_SCREEN: &str = "e0e6f052f6724f7d";
/// The team screen (retail FE 67, template 104; `MenuStage::open_team`): My Career Team's page.
const TEAM: &str = "TeamManagement";
const TEAM_PATH: &str = "TeamSkater_Selection/team_management.swf";
const TEAM_SCREEN: &str = "3823e2a9a554d38a";
/// skate_vita: the team screen's empty-name warning (82651A48, popup 0x12E).
const TEAM_NAME_POPUP: &str = "team.empty_name";
/// The Apt key AptOption (Y; AptMisc, X, is below).
const APT_OPTION: u32 = 4;
/// The browser's tab keys (retail key codes: RB 6, LB 9 the top tabs; RT 7, LT 10 the bottom ones).
const KEY_RB: u32 = 6;
const KEY_RT: u32 = 7;
const KEY_LB: u32 = 9;
const KEY_LT: u32 = 10;
/// The action overlay (research b426: template 59 SignUpHUD, `hud2/actionoverlay2`): the teleporter prompt, opened once
/// on the "popup" layer like the popups; state 2 shows it with the title (`SignUpPoster.GetTeleportTitle`), 0 hides it.
const SIGNUP: &str = "SignUpHUD";
const SIGNUP_PATH: &str = "hud2/actionoverlay2.swf";
/// The Extras template (record 15) and its path; FE screen 18's init opens it (0x82655478, b320). The pause item.
const EXTRAS: &str = "Extras";
const EXTRAS_PATH: &str = "main/extras.swf";
const EXTRAS_ITEM: &str = "Extras";
/// The text viewer template (record 28) and its path; FE screen 3's init opens it (0x8263A190, b322).
const TEXT_VIEWER: &str = "ToSScreen";
const TEXT_VIEWER_PATH: &str = "nucleus/nucleus_tos.swf";
/// The cheat dialog's popup callback (its choice changes nothing). skate_vita: the dialogs (the cheat's, the code
/// rows', the invalid characters' warning) are standard popups with their OK option; upstream raised them as small
/// popups, whose movie (`small_popup`) shows no option and takes only B on a cancelable popup, so they stayed up.
const CHEAT_POPUP: &str = "cheat_code.dialog";
/// The Skate Profile template (record 106) and its path; FE screen 61 (b308). The pause item that opens it.
const OFFLINE_PROFILE: &str = "OfflineProfile";
const OFFLINE_PROFILE_PATH: &str = "profile/offline_skateProfile.swf";
const OFFLINE_PROFILE_ITEM: &str = "SkateProfileOffline";
/// Its keys (handler 0x8264C608): the tab keys (AptRShift2 / AptLShift2) and the scroll keys left to the movie.
const PROFILE_NEXT_TAB: u32 = 7;
const PROFILE_PREVIOUS_TAB: u32 = 10;
const PROFILE_SCROLL: [u32; 4] = [12, 13, 24, 25];
/// The SpawnSkater template (record 107) and its path; FE screen 62's init opens it (0x8264D958, b543). The Free Play
/// and Call Skater pause items open it.
const SPAWN_SKATER: &str = "SpawnSkater";
const SPAWN_SKATER_PATH: &str = "cas/spawnskater.swf";
/// Its limit popup's callback (the choice changes nothing).
const SPAWN_SKATER_POPUP: &str = "spawn_skater.limit";
/// The Free Play options template (record 13) and its path; FE 34 (X in FE 62 Free Play, flow 0x8A; b548 / b564).
const FREE_SKATE: &str = "FreeSkateOption";
const FREE_SKATE_PATH: &str = "main/freeskate_options.swf";
/// The loading screen (template 131 TipsLoader, FE 52; research b581; upstream `0947dea5`), optional in the set, and
/// its lines (`vita-bake ui`).
const TIPS_LOADER: &str = "TipsLoader";
const TIPS_LOADER_PATH: &str = "main/tiploader.swf";
pub const TIPS_LOADER_DATA: &str = "private/tips-loader.json";
/// The fader bit the game state's loading holds (sub_826E08A8 bit 2).
const FADE_LOAD: u32 = 2;
/// AptMisc (X) and the direction keys (pad, left stick, d-pad repeats) FE 34 reads.
const APT_MISC: u32 = 5;
const KEYS_UP: [u32; 3] = [12, 16, 24];
const KEYS_DOWN: [u32; 3] = [13, 17, 25];
const KEYS_LEFT: [u32; 3] = [14, 18, 26];
const KEYS_RIGHT: [u32; 3] = [15, 19, 27];
/// skate_vita: the Licenses row (upstream greys it as WIP; ours shows this project's licenses, user 2026-10-23).
const LICENSES_ROW: &str = "extras.licenses";
const CROSSBAR_ENUMS: [&str; 6] =
    ["RefreshMenu", "RefreshOption", "InitialiseMenu", "InitialiseSubOption", "InitSubOptionOnlineTeam", "RefreshDLCBanner"];
const POPUP_KINDS: [PopupKind; 3] = [PopupKind::Standard, PopupKind::Small, PopupKind::AutoSave];

/// skate_vita (memory): the movies the pause front end runs on, for `RetailMenuMovies::read_lean` (their imports
/// come along): the boot root and what it loads (the screen manager, the chyron, the dimmer, ...), the pause menu,
/// Game Settings, Video Calibration, the Trick Guide, the Challenge Map, the instant replay screen and its help, the
/// action overlay (the teleporter prompt), Extras, the text viewer, the Skate Profile, SpawnSkater, the Free Play
/// options, the loading screen, the popups and the Object Dropper and its Quick Menu. Not the HUD movies.
pub const PAUSE_ROOTS: [&str; 32] = [
    "source/screens/main",
    "source/screens/main/FE_root",
    "source/screens/main/chyron",
    "source/screens/main/subtitle",
    "source/screens/main/fmv_popup",
    "source/screens/main/dimmer",
    "source/controls/fadebackground",
    "source/controls/autosave",
    "source/helper/screenmanager",
    "source/helper/screentransitionmanager",
    "source/screens/main/core_menu",
    "source/screens/options/options",
    "source/screens/options/videocal",
    "source/screens/tricks/trickguide",
    "source/screens/map/challengemap",
    "source/screens/replay2/replay_editor2",
    "source/screens/replay2/replay_help",
    "source/screens/hud2/actionoverlay2",
    "source/screens/main/extras",
    "source/screens/nucleus/nucleus_tos",
    "source/screens/profile/offline_skateProfile",
    "source/screens/cas/spawnskater",
    "source/screens/main/freeskate_options",
    "source/screens/main/tiploader",
    "source/screens/popup/sk8popup",
    "source/screens/popup/small_popup",
    "source/screens/objectdropper/objectdropper",
    "source/screens/objectdropper/objectquickmenu",
    "source/screens/browser/browser",
    "source/screens/skatepark/parksetup",
    "source/screens/cas/createskater",
    "source/screens/TeamSkater_Selection/team_management",
];

/// What runs when the screen is black (the fade hold's callback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FadeAction {
    Push(usize),
    Close(usize),
}

/// HUD visibility from the FE screen stack (info record +8 of the main stack's top, sub_82D098F0 -> vt+44): `hud` =
/// the gameplay HUD, `online` = the online HUDs. An empty stack means both on (upstream `fe_backdrop::FeHud`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeHud {
    pub hud: bool,
    pub online: bool,
}
impl Default for FeHud {
    fn default() -> Self {
        Self { hud: true, online: true }
    }
}

/// The menu structure before the registry is built ([`MenuStage::boot_with`]): the retail data, the sub-option lists
/// by kind and the settings screens' titles by screen.
pub struct Customize<'a> {
    pub data: &'a mut skate_core::menus::MenuData,
    pub sub_options: &'a mut Vec<SubOptionRow>,
    pub settings_titles: &'a mut Vec<String>,
}

/// What a frame of the front end asks of the game.
#[derive(Debug, Default)]
pub struct Frame {
    /// FE sound records to play (fe record names: `core_a_button`, ...), in order.
    pub sounds: Vec<String>,
    /// Settings rows the player stepped or selected, with the registry's outcome; the game applies the new values
    /// (`menus.registry.value`).
    pub settings: Vec<(String, Outcome)>,
    /// Pause items selected that no ported FE screen took, with the registry's outcome.
    pub items: Vec<(String, Outcome)>,
    /// skate_vita: sub-option rows chosen (A in an item's column: the item id and the row), for the game.
    pub sub_options: Vec<(String, usize)>,
    /// Challenge Map locations picked (fe_locations keys): the game teleports there ([`MenuStage::teleport`]).
    pub teleports: Vec<String>,
    /// What happened (screen changes, keys, errors), for the session log.
    pub log: Vec<String>,
    /// Choices made in the game's popups ([`MenuStage::push_popup`]): the request's callback and the option index
    /// (B on a cancelable one: -1 from the movie, or `skate_core::fe_popup::CHOICE_CANCEL`).
    pub popup_choices: Vec<(String, i32)>,
    /// skate_vita: what the team screen asks the game for (the customisation, a keyboard).
    pub team_effects: Vec<skate_core::team::Effect>,
    /// skate_vita: the text entry (FE 46) asks for text: the title as text and the request. The game opens the
    /// system keyboard and answers with [`MenuStage::text_entered`] (retail: `XamShowKeyboardUI`, b305).
    pub text_entry: Option<(String, skate_core::text_entry::TextEntryRequest)>,
    /// Cheats turned on or off (`skate_core::cheats` ids), for the game to apply.
    pub cheats: Vec<(String, bool)>,
    /// Unlocks granted (Extras rows 1-3), for the game's unlock store (`skate_core::unlocks`).
    pub unlocks: Vec<(skate_core::unlocks::UnlockKind, String)>,
    /// What FE 62 asked of the game: skaters called or dismissed, the player's skater picked.
    pub spawn_skater: Vec<skate_core::spawn_skater::Action>,
    /// FE 34 closed with these Free Play values (flow 0x8B): the game applies them.
    pub free_play: Option<skate_core::living_world::FreePlay>,
}

pub struct MenuStage {
    stage: Stage,
    movies: RetailMenuMovies,
    pub menus: RetailMenus,
    language: Arc<BTreeMap<String, String>>,
    /// The set's shadow font, for the HUD phone's rows the Vita lays itself.
    shadow_font: Option<crate::fe::apt_text::Font>,
    sub_options: Arc<Vec<SubOptionRow>>,
    /// Retail screens in id order (menu tables).
    screens: Vec<FeScreen>,
    crossbar: CrossbarState,
    rules: CrossbarRules,
    /// The FE screen stack (skate_core::fe_stack).
    stack: FeStack,
    /// A stage error stopped the front end (gameplay given back); the game drops it.
    pub failed: bool,
    /// skate_vita: Back and Start close the pause menu (false for a menu that stays, the Vita's main menu).
    pub closable: bool,
    /// The button prompts of the last draw (CustomButton `prompt/<retail button>`): the retail name and the rectangle
    /// (centre x, centre y, width, height, stage space), for the host to draw its own controller glyphs.
    prompts: Vec<(String, [f32; 4])>,
    /// AS screen ids (600 + template index): the pause menu (retail 669), Game Settings (690), Video Calibration
    /// (691).
    crossbar_id: i32,
    options_id: i32,
    video_cal_id: i32,
    /// The Trick Guide screen's AS id (retail 629), its host (empty guide = not available) and the ticks left before
    /// its fade-in releases (None = released / not waiting).
    trick_guide_id: i32,
    trick_guide: crate::fe::trick_guide_host::TrickGuideHost,
    trick_guide_fade_in: Option<u32>,
    /// The Challenge Map screen's AS id, its host and the locations' teleports (map, locator matrix).
    challenge_map_id: i32,
    challenge_map: crate::fe::challenge_map_host::ChallengeMapHost,
    teleports: BTreeMap<String, (String, [[f32; 4]; 4])>,
    /// The instant replay screen's and its help's AS ids (None: no template or movie in this install), its host, and
    /// whether it is shown (after its IntroComplete).
    replay_id: Option<i32>,
    replay_help_id: Option<i32>,
    replay_menu: crate::fe::replay_menu_host::ReplayMenuHost,
    replay_shown: bool,
    /// The game's replay this frame (`Self::set_replay`): Some while a replay is open.
    replay_feed: Option<crate::fe::replay_menu_host::ReplayScreenFeed>,
    /// The Object Dropper screen's AS id (None: no template or movie in this install), its host, the editor this
    /// frame (`Self::set_object_dropper`: Some while it is open) and the screen modes it posted since the last update.
    object_dropper_id: Option<i32>,
    /// The browser's movie id (its template registered and movie in the set), the host between updates, and the
    /// game's requests: open with these parks, new parks.
    browser_id: Option<i32>,
    browser: crate::fe::browser_host::BrowserHost,
    browser_open: Option<Vec<crate::fe::browser_host::BrowserItem>>,
    browser_items: Option<Vec<crate::fe::browser_host::BrowserItem>>,
    /// Park Setup's movie id, the host between updates, and the game's open request (its lots).
    park_setup_id: Option<i32>,
    park_setup: crate::fe::park_setup_host::ParkSetupHost,
    park_setup_open: Option<Vec<crate::fe::park_setup_host::ParkSetupLot>>,
    cac_id: Option<i32>,
    cac: crate::fe::cac_host::CacHost,
    cac_open: Option<(skate_core::cac::Cac, u32)>,
    team_id: Option<i32>,
    team: crate::fe::team_host::TeamHost,
    team_open: Option<skate_core::team::Team>,
    /// The left stick or d-pad across, -1 to 1, this frame.
    cac_axis: f32,
    /// LT minus RT this frame.
    cac_turn: f32,
    /// The placement page's inputs: the turn, the scale, the offsets (825FBAE8).
    cac_adjust: [f32; 4],
    object_quick_menu_id: Option<i32>,
    object_dropper: crate::fe::object_dropper_host::ObjectDropperHost,
    object_dropper_feed: Option<crate::fe::object_dropper_host::ObjectDropperFeed>,
    object_dropper_posts: Vec<u8>,
    object_quick_posts: Vec<u8>,
    /// The action overlay's AS id (None: no movie in the set), the title the game asks it to show and the one shown.
    signup_id: Option<i32>,
    signup_title: Option<String>,
    /// The autosave layer's save wheel: wanted by the game ([`MenuStage::set_save_tag`]), and as last applied.
    save_tag: bool,
    save_tag_shown: bool,
    signup_shown: Option<String>,
    /// The Extras screen's and the text viewer's AS ids, their hosts and the cheat table.
    extras_id: i32,
    viewer_id: i32,
    /// The Skate Profile screen's AS id and host.
    profile_id: i32,
    offline_profile: crate::fe::offline_profile_host::OfflineProfileHost,
    extras: crate::fe::extras_host::ExtrasHost,
    tos: crate::fe::extras_host::TosHost,
    /// FE 62's AS id, its host, the mode the next push opens it in (the pause item picks it), the pros Call Skater
    /// lists and the skaters the game has called ([`Self::set_called_skaters`]).
    spawn_skater_id: i32,
    spawn_skater: crate::fe::spawn_skater_host::SpawnSkaterHost,
    spawn_skater_mode: skate_core::spawn_skater::Mode,
    callable: Vec<crate::fe::spawn_skater_host::Callable>,
    called: Vec<String>,
    /// FE 34's AS id, its host and the game's Free Play values it opens with ([`Self::set_free_play`]).
    free_skate_id: i32,
    free_skate: crate::fe::free_skate_options_host::FreeSkateOptionsHost,
    free_play: skate_core::living_world::FreePlay,
    /// FE 52's AS id (None: no movie or template in the set), its `TipsLoader` host, the load sequence, whether the
    /// screen's intro is done / it closed, and whether the game is loading ([`Self::set_loading`]).
    tips_loader_id: Option<i32>,
    tips_loader: crate::fe::tips_loader_host::TipsLoaderHost,
    load: skate_core::tips_loader::LoadScreen,
    load_up: bool,
    load_gone: bool,
    loading: bool,
    /// The career stats table (menu tables `career_stats`; empty: no Stats panel), the game's totals and its Units
    /// setting ([`Self::set_career_stats`]).
    career_table: Vec<skate_data::menu_tables::StatRow>,
    career_stats: skate_core::career_stats::CareerStats,
    imperial: bool,
    cheats: skate_core::cheats::CheatRules,
    /// The cheats in effect (the game's flags, [`Self::set_cheats`]): entering a code toggles them.
    cheats_on: skate_core::cheats::CheatState,
    /// The text entry FE 46 opens next, the one it shows, whether the game must be asked again (an empty entry, the
    /// warning popup's OK), and the game's answer (None in the inner option: cancelled).
    pending_entry: Option<skate_core::text_entry::TextEntryRequest>,
    entry: Option<skate_core::text_entry::TextEntryRequest>,
    entry_ask: bool,
    entry_answer: Option<Option<String>>,
    /// The chyron's visibility as last set (vt+48 acts on changes only).
    chyron: bool,
    settings: crate::fe::settings_host::SettingsState,
    settings_titles: Vec<String>,
    image_slots: crate::fe::image_renderer_host::ImageSlots,
    /// The FE fader (fade to black around screen changes).
    fade: FeFade<FadeAction>,
    fade_params: FeFadeParams,
    fade_carry: f32,
    video_cal: VideoCalibration,
    video_cal_rules: CalibrationRules,
    /// The popup list, the host's state and the popup movies' AS ids by kind (preloaded on the "popup" layer).
    /// `popup_dirty`: show the last entry again (FE vt+24 runs on every push and after a removal).
    popups: PopupList,
    popup: crate::fe::popup_host::PopupHost,
    popup_ids: Vec<(PopupKind, i32)>,
    popup_dirty: bool,
    /// Retail Apt keys in key-table order (input.cfg expressions).
    apt_keys: Vec<AptKey>,
    pads: [FeInput; 4],
    repeat: RepeatRule,
    /// Gameplay suspended by the stack (a screen whose record suspends it is open).
    suspended: bool,
    hud: FeHud,
}

/// One stack batch can open a screen and close it again (op 5 unwinds and reopens): the pair is dropped, the screen
/// ends closed either way (upstream `drop_reopen_and_close`; NOT RETAIL (mechanism)).
fn drop_reopen_and_close(effects: Vec<Effect>) -> Vec<Effect> {
    let mut keep = vec![true; effects.len()];
    for (i, effect) in effects.iter().enumerate() {
        if let Effect::Open(screen) = *effect {
            if let Some(j) = effects[i + 1..].iter().position(|e| *e == Effect::Close(screen)) {
                keep[i] = false;
                keep[i + 1 + j] = false;
            }
        }
    }
    effects.into_iter().zip(keep).filter_map(|(e, k)| k.then_some(e)).collect()
}

fn tables_of(read: Reader) -> Option<MenuTables> {
    let bytes = read(TABLES).ok()?;
    MenuTables::from_json(std::str::from_utf8(&bytes).ok()?).ok()
}

/// Retail ScreenManagerReady handler (825D9450), in its order, with values as "%d" strings like retail: the
/// templates in table order, the 56 Apt keys (value = key index, the index UpdateInput passes), FE_SOUND_<NAME> =
/// "<name>" per fe record, then platform "0", screen size 1280 x 720, edge offset "136", PAL mode "false".
fn boot_registration(stage: &mut Stage, fe: &mut FrontEnd, tables: Option<&MenuTables>, read: Reader) -> Result<Vec<String>, String> {
    let text = |s: &str| Value::Text(s.into());
    let names: Vec<String> = match tables.map(|t| &t.templates).filter(|t| !t.is_empty()) {
        Some(templates) => templates.iter().map(|t| t.name.clone()).collect(),
        None => vec![CROSSBAR.into()],
    };
    for name in &names {
        stage.call_path("_global.ScreenManager.RegisterTemplate", vec![text(name)], fe)?;
    }
    for (i, key) in tables.iter().flat_map(|t| &t.keys).enumerate() {
        stage.call_path("_level0._root.RegisterAptInputEnum", vec![text(&key.name), text(&i.to_string())], fe)?;
    }
    for name in fe_sound_names(read) {
        stage.call_path(
            "_level0._root.RegisterAudioEnum",
            vec![text(&format!("FE_SOUND_{}", name.to_ascii_uppercase())), text(&name)],
            fe,
        )?;
    }
    stage.call_path("_level0._root.SetPlatform", vec![text("0")], fe)?;
    stage.call_path("_level0._root.SetScreenSize", vec![text("1280"), text("720")], fe)?;
    // No SetScreenEdgeOffset: retail sends its SD value "136" only without the HD flag (doc 31 "Screen edge offset").
    stage.call_path("_level0._root.UpdatePALMode", vec![text("false")], fe)?;
    Ok(names)
}

/// skate_vita: the fe sound records' names (`frontend.sounds.*.name` of the audio manifest), sorted.
/// [`FE_SOUNDS`] (vita-bake writes the list) when the assets have it, else the manifest, read for that field only
/// (upstream parsed the whole 17 MB manifest into a JSON tree: a 68 MB peak at boot).
fn fe_sound_names(read: Reader) -> Vec<String> {
    #[derive(serde::Deserialize, Default)]
    struct Manifest {
        #[serde(default)]
        frontend: Frontend,
    }
    #[derive(serde::Deserialize, Default)]
    struct Frontend {
        #[serde(default)]
        sounds: BTreeMap<String, Sound>,
    }
    #[derive(serde::Deserialize)]
    struct Sound {
        name: Option<String>,
    }
    let mut names: Vec<String> = match read(FE_SOUNDS) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => read(AUDIO_MANIFEST)
            .ok()
            .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
            .map(|m| m.frontend.sounds.into_values().filter_map(|s| s.name).collect())
            .unwrap_or_default(),
    };
    names.sort_unstable();
    names
}

/// The Apt keys: names from the menu tables' key table, expressions from the stock input.cfg.
fn apt_keys(tables: Option<&MenuTables>, read: Reader) -> Result<Vec<AptKey>, String> {
    let names: Vec<String> = tables.map(|t| t.keys.iter().map(|k| k.name.clone()).collect()).unwrap_or_default();
    let cfg = read(skate_data::input_config::STOCK_INPUT_PATH).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    parse_apt_keys(&cfg, &names).map_err(|e| format!("input.cfg: {e}"))
}

/// The menu tables register this template.
fn templates_have(tables: Option<&MenuTables>, template: &str) -> bool {
    tables.is_some_and(|t| t.templates.iter().any(|x| x.name == template))
}

/// A popup template's path from the menu tables' template list (`popup/sk8popup.swf`).
fn popup_path<'t>(tables: Option<&'t MenuTables>, template: &str) -> Option<&'t str> {
    tables?.templates.iter().find(|t| t.name == template).map(|t| t.path.as_str())
}

fn movie_key(path: &str) -> String {
    format!("source/screens/{}", path.trim_end_matches(".swf"))
}

fn front_end<'a>(
    menus: &'a RetailMenus,
    language: &'a BTreeMap<String, String>,
    sub_options: &Arc<Vec<SubOptionRow>>,
    crossbar: CrossbarState,
) -> FrontEnd<'a> {
    let mut fe = FrontEnd::new(&menus.registry, &menus.context, language);
    fe.state = crossbar;
    fe.sub_options = Arc::clone(sub_options);
    let counts = crate::fe::crossbar_host::sub_option_counts(&fe.sub_options, &menus.context);
    fe.view = skate_core::crossbar::RegistryView::new(&menus.registry, &menus.context, counts);
    fe
}

impl MenuStage {
    /// Retail boot (sub_825D7E10) on `movies` (a set holding at least `main`, `core_menu` and the screens' movies):
    /// screens/main into _level0, the screen manager's ready call answered with the boot registration, the crossbar
    /// enums, the popup movies preloaded. `read` gives the menu tables, the audio manifest and input.cfg.
    pub fn boot(movies: RetailMenuMovies, read: Reader) -> Result<Self, String> {
        Self::boot_with(movies, read, &mut |_| {})
    }

    /// [`Self::boot`] with the host's changes to the menu structure before the registry is built (skate_vita: the
    /// Vita's own items, sub-option lists and settings screens; handlers and values go on `menus.registry` after).
    pub fn boot_with(movies: RetailMenuMovies, read: Reader, customize: &mut dyn FnMut(Customize)) -> Result<Self, String> {
        let mut tables = tables_of(read);
        let (mut data, loaded) = tables.as_ref().map_or_else(Default::default, |t| (t.to_menu_data(), true));
        let mut sub_options = tables.as_ref().map(|t| t.sub_options.clone()).unwrap_or_default();
        let mut settings_titles = tables.as_mut().map(|t| std::mem::take(&mut t.settings_titles)).unwrap_or_default();
        let career_table = tables.as_mut().map(|t| std::mem::take(&mut t.career_stats)).unwrap_or_default();
        // The Challenge Map's tree, pictures and markers (vita-bake), when the movie is in the set.
        let map = read(CHALLENGE_MAP_DATA)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| skate_data::challenge_map::Baked::from_json(&v).ok())
            .filter(|_| movies.movies.contains_key(&movie_key(CHALLENGE_MAP_PATH)))
            .unwrap_or_default();
        data.map_nodes = map.nodes;
        data.map_pictures = map.pictures;
        data.map_markers = map.markers;
        data.map_debug = map.debug;
        let teleports = map.teleports;
        customize(Customize { data: &mut data, sub_options: &mut sub_options, settings_titles: &mut settings_titles });
        let mut menus = RetailMenus::new(data, loaded);
        let language = movies.language();
        let shadow_font = movies.shadow_font();
        let sub_options = Arc::new(sub_options);
        let mut fe = front_end(&menus, &language, &sub_options, CrossbarState::default());
        let mut stage = Stage::new(&fe)?;
        // Screens/main into _level0; its root loads the screen manager into gScreenManager and the HUD / autosave /
        // fmv levels, which land on the next updates.
        stage.load_root("source/screens/main", &movies.movies, &mut fe)?;
        for _ in 0..4 {
            stage.update(16.0, &movies.movies, &mut fe)?;
        }
        let mut templates = vec![CROSSBAR.to_owned()];
        if fe.calls.iter().any(|c| c == "Game.ScreenManagerReady()") {
            templates = boot_registration(&mut stage, &mut fe, tables.as_ref(), read)?;
        }
        // Crossbar enums (sub_8261B9C8): RegisterChallengeEnum(name, "%d"), value = index in table 0x830274F0.
        for (i, name) in CROSSBAR_ENUMS.iter().enumerate() {
            stage.call_path(
                "_level0._root.RegisterChallengeEnum",
                vec![Value::Text(format!("FECrossBarScreenMode_{name}")), Value::Text(i.to_string())],
                &mut fe,
            )?;
        }
        // The Object Dropper's screen modes, the same way (their C++ name table `0x8302C440`), when its movie is here.
        if movies.movies.contains_key(&movie_key(OBJECT_DROPPER_PATH)) {
            for (i, name) in crate::fe::object_dropper_host::SCREEN_MODES.iter().enumerate() {
                stage.call_path(
                    "_level0._root.RegisterChallengeEnum",
                    vec![Value::Text(format!("FEObjectDropperScreenMode_{name}")), Value::Text(i.to_string())],
                    &mut fe,
                )?;
            }
        }
        // The browser's modes (names `0x8302792C`).
        if movies.movies.contains_key(&movie_key(BROWSER_PATH)) {
            for (i, name) in crate::fe::browser_host::MODES.iter().enumerate() {
                stage.call_path(
                    "_level0._root.RegisterChallengeEnum",
                    vec![Value::Text(format!("FECommonBrowserMode_{name}")), Value::Text(i.to_string())],
                    &mut fe,
                )?;
            }
        }
        if movies.movies.contains_key(&movie_key(OBJECT_QUICK_MENU_PATH)) {
            for (i, name) in crate::fe::object_dropper_host::QUICK_SCREEN_MODES.iter().enumerate() {
                stage.call_path(
                    "_level0._root.RegisterChallengeEnum",
                    vec![Value::Text(format!("FEObjectQuickMenuScreenMode_{name}")), Value::Text(i.to_string())],
                    &mut fe,
                )?;
            }
        }
        // Popup host create (FE screen 29 vt+4, 0x825EF370): templates 17 Sk8Popup, 18 Sk8SmallPopup and 4
        // Sk8AutoSavePopup open once on the AS layer "popup"; they stay loaded and are shown / hidden per popup.
        let popup_movies: Vec<(PopupKind, String)> = POPUP_KINDS
            .into_iter()
            .filter_map(|k| popup_path(tables.as_ref(), k.template()).map(|p| (k, p.to_owned())))
            .filter(|(_, p)| movies.movies.contains_key(&movie_key(p)))
            .collect();
        for (kind, path) in &popup_movies {
            open_screen_on(&mut stage, path, "popup", kind.template(), &mut fe)?;
        }
        // The action overlay: opened once on the popup layer, hidden until a teleporter shows it (state 2).
        let has_signup = templates_have(tables.as_ref(), SIGNUP) && movies.movies.contains_key(&movie_key(SIGNUP_PATH));
        if has_signup {
            open_screen_on(&mut stage, SIGNUP_PATH, "popup", SIGNUP, &mut fe)?;
        }
        drop(fe);
        // AS screen ids: 600 + the template's registration index.
        let as_id = |name: &str| 600 + templates.iter().position(|n| n == name).unwrap_or(0) as i32;
        let popup_ids = popup_movies.iter().map(|(k, _)| (*k, as_id(k.template()))).collect();
        // Video Calibration (FE 51): the settings row gets its engine handler when the movie is in the set.
        if movies.movies.contains_key(&movie_key(VIDEO_CALIBRATION_PATH))
            && menus.registry.data().settings.iter().any(|s| s.id == VIDEO_CALIBRATION_ROW)
        {
            let owner = Owner::Engine(format!("{SCREEN}{VIDEO_CALIBRATION_SCREEN}"));
            let handler = Handler { owner, select: true, highlight: false, callback: None };
            menus.registry.register_handler(EntryRef::setting(VIDEO_CALIBRATION_ROW), handler);
        }
        let apt_keys = apt_keys(tables.as_ref(), read)?;
        // The Trick Guide (FE 14): its data (vita-bake) and its movie in the set; the pause item gets its engine
        // handler (greyed without them).
        let guide = read(TRICK_GUIDE_DATA)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| skate_data::trick_guide::from_json(&v).ok())
            .filter(|g| !g.categories.is_empty() && movies.movies.contains_key(&movie_key(TRICK_GUIDE_PATH)));
        let trick_guide = match guide {
            Some(guide) => {
                if menus.registry.data().item(TRICK_GUIDE_ITEM).is_some() {
                    let owner = Owner::Engine(format!("{SCREEN}{TRICK_GUIDE_SCREEN}"));
                    menus.registry.register_handler(EntryRef::item(TRICK_GUIDE_ITEM), Handler { owner, select: true, highlight: false, callback: None });
                }
                crate::fe::trick_guide_host::TrickGuideHost::new(Arc::new(guide))
            }
            None => Default::default(),
        };
        // The Challenge Map (FE 17): the pause item opens the retail screen when its movie and data are there.
        if !menus.registry.data().map_nodes.is_empty() && menus.registry.data().item(CHALLENGE_MAP_ITEM).is_some() {
            let owner = Owner::Engine(format!("{SCREEN}{CHALLENGE_MAP_SCREEN}"));
            menus.registry.register_handler(EntryRef::item(CHALLENGE_MAP_ITEM), Handler { owner, select: true, highlight: false, callback: None });
        }
        // The instant replay screen and its help: their templates registered and their movies in the set.
        let has = |name: &str, path: &str| templates.iter().any(|n| n == name) && movies.movies.contains_key(&movie_key(path));
        // Extras (FE 18): the pause item opens the retail screen when its movie is in the set.
        if has(EXTRAS, EXTRAS_PATH) && menus.registry.data().item(EXTRAS_ITEM).is_some() {
            let owner = Owner::Engine(format!("{SCREEN}{EXTRAS_SCREEN}"));
            menus.registry.register_handler(EntryRef::item(EXTRAS_ITEM), Handler { owner, select: true, highlight: false, callback: None });
        }
        // The Skate Profile (FE 61): the same.
        if has(OFFLINE_PROFILE, OFFLINE_PROFILE_PATH) && menus.registry.data().item(OFFLINE_PROFILE_ITEM).is_some() {
            let owner = Owner::Engine(format!("{SCREEN}{OFFLINE_PROFILE_SCREEN}"));
            menus.registry.register_handler(EntryRef::item(OFFLINE_PROFILE_ITEM), Handler { owner, select: true, highlight: false, callback: None });
        }
        // SpawnSkater (FE 62): Free Play (SoloFreeskateOptions) and Call Skater (SkateWith) push it (b307); greyed
        // without the movie (sets before version 14).
        if has(SPAWN_SKATER, SPAWN_SKATER_PATH) {
            for (item, _) in skate_core::spawn_skater::SpawnSkaterRules::default().items {
                if menus.registry.data().item(&item).is_some() {
                    let owner = Owner::Engine(format!("{SCREEN}{SPAWN_SKATER_SCREEN}"));
                    menus.registry.register_handler(EntryRef::item(&item), Handler { owner, select: true, highlight: false, callback: None });
                }
            }
        }
        let mut extras = crate::fe::extras_host::ExtrasHost::default();
        if let Some(row) = extras.rules.rows.iter_mut().find(|r| r.id == LICENSES_ROW) {
            row.greyed = !has(TEXT_VIEWER, TEXT_VIEWER_PATH);
        }
        let replay_id = has(REPLAY, REPLAY_PATH).then(|| as_id(REPLAY));
        let object_dropper_id = has(OBJECT_DROPPER, OBJECT_DROPPER_PATH).then(|| as_id(OBJECT_DROPPER));
        let browser_id = has(BROWSER, BROWSER_PATH).then(|| as_id(BROWSER));
        let park_setup_id = has(PARK_SETUP, PARK_SETUP_PATH).then(|| as_id(PARK_SETUP));
        let cac_id = has(CAC, CAC_PATH).then(|| as_id(CAC));
        let team_id = has(TEAM, TEAM_PATH).then(|| as_id(TEAM));
        let object_quick_menu_id = has(OBJECT_QUICK_MENU, OBJECT_QUICK_MENU_PATH).then(|| as_id(OBJECT_QUICK_MENU));
        // The loading screen's lines; without them its panels have nothing to say (Coach Frank and the hint).
        let tips_lines: skate_data::tips_loader::TipsLines =
            read(TIPS_LOADER_DATA).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let tips_loader_id = (has(TIPS_LOADER, TIPS_LOADER_PATH) && !tips_lines.coach.is_empty()).then(|| as_id(TIPS_LOADER));
        let replay_help_id = has(REPLAY_HELP, REPLAY_HELP_PATH).then(|| as_id(REPLAY_HELP));
        let signup_id = has_signup.then(|| as_id(SIGNUP));

        Ok(Self {
            stage,
            movies,
            menus,
            language,
            shadow_font,
            sub_options,
            screens: tables.as_ref().map(|t| t.screens.clone()).unwrap_or_default(),
            crossbar: CrossbarState::default(),
            rules: CrossbarRules::default(),
            stack: FeStack::default(),
            failed: false,
            closable: true,
            prompts: Vec::new(),
            crossbar_id: as_id(CROSSBAR),
            options_id: as_id(OPTIONS),
            video_cal_id: as_id(VIDEO_CALIBRATION),
            trick_guide_id: as_id(TRICK_GUIDE),
            trick_guide,
            trick_guide_fade_in: None,
            challenge_map_id: as_id(CHALLENGE_MAP),
            challenge_map: Default::default(),
            teleports,
            replay_id,
            replay_help_id,
            replay_menu: Default::default(),
            object_dropper_id,
            browser_id,
            browser: Default::default(),
            browser_open: None,
            browser_items: None,
            park_setup_id,
            park_setup: Default::default(),
            park_setup_open: None,
            cac_id,
            cac: Default::default(),
            cac_open: None,
            team_id,
            team: Default::default(),
            team_open: None,
            cac_axis: 0.0,
            cac_turn: 0.0,
            cac_adjust: [0.0; 4],
            object_quick_menu_id,
            object_dropper: Default::default(),
            object_dropper_feed: None,
            object_dropper_posts: Vec::new(),
            object_quick_posts: Vec::new(),
            replay_shown: false,
            replay_feed: None,
            signup_id,
            signup_title: None,
            save_tag: false,
            save_tag_shown: false,
            signup_shown: None,
            extras_id: as_id(EXTRAS),
            viewer_id: as_id(TEXT_VIEWER),
            profile_id: as_id(OFFLINE_PROFILE),
            offline_profile: Default::default(),
            extras,
            tos: Default::default(),
            spawn_skater_id: as_id(SPAWN_SKATER),
            spawn_skater: Default::default(),
            spawn_skater_mode: Default::default(),
            callable: Vec::new(),
            called: Vec::new(),
            free_skate_id: as_id(FREE_SKATE),
            free_skate: Default::default(),
            free_play: Default::default(),
            tips_loader_id,
            // The tips RNG's seed (retail's is open): the boot's time, as upstream.
            tips_loader: crate::fe::tips_loader_host::TipsLoaderHost {
                lines: tips_lines,
                seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() | 1),
                ..Default::default()
            },
            load: Default::default(),
            load_up: false,
            load_gone: false,
            loading: false,
            career_table,
            career_stats: Default::default(),
            imperial: false,
            cheats: Default::default(),
            cheats_on: Default::default(),
            pending_entry: None,
            entry: None,
            entry_ask: false,
            entry_answer: None,
            chyron: false,
            settings: Default::default(),
            settings_titles,
            image_slots: Default::default(),
            fade: Default::default(),
            fade_params: Default::default(),
            fade_carry: 0.0,
            video_cal: Default::default(),
            video_cal_rules: Default::default(),
            popups: Default::default(),
            popup: Default::default(),
            popup_ids,
            popup_dirty: false,
            apt_keys,
            pads: Default::default(),
            repeat: RepeatRule::RETAIL,
            suspended: false,
            hud: FeHud::default(),
        })
    }

    /// Closes every screen on the main stack, top first (the game leaves the menus: a park to load, a respawn).
    pub fn close(&mut self) {
        let screens: Vec<usize> = self.stack.screens(Layer::Main).collect();
        for screen in screens.into_iter().rev() {
            self.stack.request(Layer::Main, Request::Close { screen });
        }
    }

    /// Replaces the rows of a sub-option list (`kind`: the items' `sub_option_kind`), e.g. lists that change while
    /// the game runs.
    pub fn set_sub_options(&mut self, kind: usize, row: SubOptionRow) {
        let rows = Arc::make_mut(&mut self.sub_options);
        if rows.len() <= kind {
            rows.resize_with(kind + 1, || SubOptionRow { labels: Vec::new(), helpers: Vec::new() });
        }
        rows[kind] = row;
    }

    /// This build's index of a screen id (the FE id in its screen table).
    fn index(&self, id: &str) -> Option<usize> {
        self.screens.iter().position(|s| s.id == id)
    }

    /// The pause menu can open: its FE screen record is in the tables.
    pub fn has_pause(&self) -> bool {
        self.index(PAUSE_SCREEN).is_some()
    }

    /// A screen is on the main stack (the pause menu or one opened from it).
    pub fn open(&self) -> bool {
        self.stack.top(Layer::Main).is_some()
    }

    /// Gameplay is suspended by an open screen (context 3).
    pub fn suspended(&self) -> bool {
        self.suspended
    }

    pub fn hud(&self) -> FeHud {
        self.hud
    }

    /// The world behind blurs: the main stack's top screen record has the blur flag (the ramp is `fe_blur`).
    pub fn blur(&self) -> bool {
        self.stack.top(Layer::Main).is_some_and(|i| self.screens.get(i).is_some_and(|s| s.blur))
    }

    /// The movies (texture lookup for [`Self::draw`]'s keys: `movies().image(key)`).
    pub fn movies(&self) -> &RetailMenuMovies {
        &self.movies
    }

    /// The VM's object count and the loaded levels (memory checks).
    pub fn object_summary(&self) -> String {
        self.stage.object_summary()
    }

    /// The keys fired this frame per controller (FE poll sub_825DFDC8: every connected controller, keys in table
    /// order); `pads` are the 24 native values per controller (`skate_core::input::xbox::convert` layout).
    fn poll_keys(&mut self, pads: &[[f32; 24]], seconds: f32) -> Vec<(u32, usize)> {
        let mut out = Vec::new();
        for (controller, values) in pads.iter().enumerate().take(4) {
            let edges = self.pads[controller].update(values, seconds, &self.repeat);
            out.extend(fired(&self.apt_keys, &edges).into_iter().map(|k| (k, controller)));
        }
        out
    }

    /// Follows the pads without acting on them (the keys are dropped): a host that hands the pads over only at times
    /// (the replay's popup) keeps the press tracking current, so the press that raised a popup is not taken by it.
    pub fn track_pads(&mut self, pads: &[[f32; 24]], seconds: f32) {
        self.poll_keys(pads, seconds);
    }

    /// One frame of `seconds` (real time: the menus run while gameplay is suspended) with the pads' values.
    pub fn update(&mut self, pads: &[[f32; 24]], seconds: f32) -> Frame {
        // The customisation's sliders move while the left stick or the d-pad is held (825FBAE8).
        self.cac_axis = pads.first().map_or(0.0, |p| {
            let dpad = p[3] - p[2];
            if dpad != 0.0 { dpad } else { p[16] }
        });
        // And the triggers turn the skater (LT one way, RT the other).
        self.cac_turn = pads.first().map_or(0.0, |p| p[10] - p[11]);
        // The tattoo placement page: the bumpers turn (TopLeftShoulderHeld minus TopRightShoulderHeld), the left stick's
        // up minus down scales, the right stick moves across and up (ManualRStickX, ManualRStickY; config/input.cfg).
        self.cac_adjust = pads.first().map_or([0.0; 4], |p| [p[8] - p[9], p[18] - p[19], p[20] - p[21], p[22] - p[23]]);
        let keys = self.poll_keys(pads, seconds);
        self.update_keys(keys, seconds)
    }

    /// [`Self::update`] with the Apt keys already fired (key index, controller).
    pub fn update_keys(&mut self, keys: Vec<(u32, usize)>, seconds: f32) -> Frame {
        let mut out = Frame::default();
        if self.failed {
            return out;
        }
        let info = |screens: &[FeScreen], index: usize| {
            screens.get(index).map_or_else(ScreenInfo::default, |s| ScreenInfo {
                suspends_gameplay: s.suspends_gameplay,
                chyron: s.chyron,
                blur: s.blur,
                hud: HudMode::from_retail(s.hud),
                input_to_movie: s.input_to_movie,
            })
        };
        let pause = self.index(PAUSE_SCREEN);
        let options = self.index(SETTINGS_SCREEN);
        let cal = self.index(VIDEO_CALIBRATION_SCREEN);
        let trick = self.index(TRICK_GUIDE_SCREEN);
        let map = self.index(CHALLENGE_MAP_SCREEN);
        let extras = self.index(EXTRAS_SCREEN);
        let entry = self.index(TEXT_ENTRY_SCREEN);
        let viewer = self.index(TEXT_VIEWER_SCREEN);
        let profile = self.index(OFFLINE_PROFILE_SCREEN);
        let spawn = self.index(SPAWN_SKATER_SCREEN);
        let free = self.index(FREE_SKATE_SCREEN);
        let browser = self.index(BROWSER_SCREEN).filter(|_| self.browser_id.is_some());
        let browser_id = self.browser_id.unwrap_or(0);
        let park_setup = self.index(PARK_SETUP_SCREEN).filter(|_| self.park_setup_id.is_some());
        let park_setup_id = self.park_setup_id.unwrap_or(0);
        let cac = self.index(CAC_SCREEN).filter(|_| self.cac_id.is_some());
        let cac_id = self.cac_id.unwrap_or(0);
        let team = self.index(TEAM_SCREEN).filter(|_| self.team_id.is_some());
        let team_id = self.team_id.unwrap_or(0);
        let (spawn_id, free_id) = (self.spawn_skater_id, self.free_skate_id);
        let loader = self.index(LOADING_SCREEN).filter(|_| self.tips_loader_id.is_some());
        let loader_id = self.tips_loader_id.unwrap_or(0);
        let (crossbar_id, options_id, cal_id, trick_id) = (self.crossbar_id, self.options_id, self.video_cal_id, self.trick_guide_id);
        let map_id = self.challenge_map_id;
        let (extras_id, viewer_id, profile_id) = (self.extras_id, self.viewer_id, self.profile_id);
        let front = crate::fe::profile::span(crate::fe::profile::Span::FrontEnd);
        let mut fe = front_end(&self.menus, &self.language, &self.sub_options, self.crossbar.clone());
        drop(front);
        fe.settings = std::mem::take(&mut self.settings);
        fe.settings_titles = self.settings_titles.clone();
        fe.settings.open = options.is_some() && self.stack.top(Layer::Main) == options;
        fe.image_slots = std::mem::take(&mut self.image_slots);
        fe.popup = std::mem::take(&mut self.popup);
        fe.trick_guide = std::mem::take(&mut self.trick_guide);
        fe.trick_guide.open = trick.is_some() && self.stack.top(Layer::Main) == trick;
        fe.challenge_map = std::mem::take(&mut self.challenge_map);
        fe.challenge_map.open = map.is_some() && self.stack.top(Layer::Main) == map;
        fe.replay_menu = std::mem::take(&mut self.replay_menu);
        fe.object_dropper = std::mem::take(&mut self.object_dropper);
        fe.browser = std::mem::take(&mut self.browser);
        // The game's browser requests: pushed on the stack (its movie opens with the push), new parks shown.
        if let (Some(items), Some(b)) = (self.browser_open.take(), browser) {
            fe.browser = crate::fe::browser_host::BrowserHost::open(items);
            self.stack.request(Layer::Main, Request::push(b));
        }
        if let Some(items) = self.browser_items.take() {
            fe.browser.set_items(items);
        }
        fe.park_setup = std::mem::take(&mut self.park_setup);
        if let (Some(lots), Some(p)) = (self.park_setup_open.take(), park_setup) {
            fe.park_setup = crate::fe::park_setup_host::ParkSetupHost::open(lots);
            self.stack.request(Layer::Main, Request::push(p));
        }
        fe.cac = std::mem::take(&mut self.cac);
        fe.team = std::mem::take(&mut self.team);
        if let (Some(t), Some(i)) = (self.team_open.take(), team) {
            fe.team = crate::fe::team_host::TeamHost { screen: Some(t), ..Default::default() };
            self.stack.request(Layer::Main, Request::push(i));
        }
        if let (Some((screen, cash)), Some(c)) = (self.cac_open.take(), cac) {
            fe.cac = crate::fe::cac_host::CacHost { screen: Some(screen), cash, ..Default::default() };
            self.stack.request(Layer::Main, Request::push(c));
        }
        fe.signup_title = self.signup_shown.clone();
        fe.extras = std::mem::take(&mut self.extras);
        fe.extras.open = extras.is_some() && self.stack.top(Layer::Main) == extras;
        fe.tos = std::mem::take(&mut self.tos);
        fe.offline_profile = std::mem::take(&mut self.offline_profile);
        fe.spawn_skater = std::mem::take(&mut self.spawn_skater);
        fe.spawn_skater.open = spawn.is_some() && self.stack.top(Layer::Main) == spawn;
        fe.free_skate = std::mem::take(&mut self.free_skate);
        fe.free_skate.open = free.is_some() && self.stack.top(Layer::Main) == free;
        fe.tips_loader = std::mem::take(&mut self.tips_loader);
        // No keys while a load runs (upstream: none while the sequence is active).
        let keys = if self.load.active() { Vec::new() } else { keys };
        let (replay_id, replay_help_id) = (self.replay_id, self.replay_help_id);
        let replay_feed = self.replay_feed.clone();
        let replay_shown = &mut self.replay_shown;
        let object_dropper_id = self.object_dropper_id;
        let object_dropper_feed = self.object_dropper_feed.clone();
        let object_dropper_posts = std::mem::take(&mut self.object_dropper_posts);
        let object_quick_posts = std::mem::take(&mut self.object_quick_posts);
        let object_quick_menu_id = self.object_quick_menu_id;
        // Work for the registry once the movie work is done: the crossbar item, settings steps.
        let mut selected: Option<String> = None;
        let mut sub_selected: Option<(String, usize)> = None;
        let mut steps: Vec<(String, Direction)> = Vec::new();
        let sounds = &mut out.sounds;
        let log = &mut out.log;
        let popup_choices = &mut out.popup_choices;
        let team_effects = &mut out.team_effects;
        let cheats_out = &mut out.cheats;
        let unlocks_out = &mut out.unlocks;
        let text_entry = &mut out.text_entry;
        let spawn_out = &mut out.spawn_skater;
        let free_play_out = &mut out.free_play;
        let stage = &mut self.stage;
        let stack = &mut self.stack;
        let screens = &self.screens;
        let movies = &self.movies;
        let result = (|| -> Result<(), String> {
            let keys_span = crate::fe::profile::span(crate::fe::profile::Span::Keys);
            for (key, controller) in keys {
                log.push(format!(
                    "key {} pad {controller} screen {:?}",
                    self.apt_keys.get(key as usize).map_or("?", |k| k.name.as_str()),
                    stack.input_target().and_then(|t| screens.get(t)).map(|s| s.id.as_str())
                ));
                // sub_825EC0E8 step 2: while the popup list has entries, every key goes to the popup host only; its
                // C++ handler (vt+40) returns 0, so the movie's UpdateInput gets the key once the host is ready (+49).
                if let Some((_, request)) = self.popups.shown() {
                    let handle = self.popup_ids.iter().find(|(k, _)| *k == request.kind).map(|(_, id)| *id);
                    if let (Some(handle), true) = (handle, fe.popup.input_ready) {
                        update_input(stage, handle, key, controller, &mut fe)?;
                    }
                    continue;
                }
                if key == APT_START {
                    // Retail Start (sub_825EB998, mode 0) pushes FE 56 with the plain push. NOT RETAIL YET
                    // (unverified): Start while it is open closes it.
                    match pause {
                        Some(p) if stack.active(p) && !self.closable => {}
                        Some(p) if stack.active(p) => stack.request(Layer::Main, Request::Close { screen: p }),
                        Some(p) => stack.request(Layer::Main, Request::push(p)),
                        None => {}
                    }
                    continue;
                }
                // The Object Dropper's Quick Menu (FE 0x36, opened by the engine, not on the stack): on its Style and
                // Branding pages the C++ handler (825CF360) leaves the keys to the movie's list (its UpdateInput,
                // `mbAllowMenuPickerInput`).
                {
                    use crate::fe::object_dropper_host::{PAGE_BRANDING, PAGE_STYLE};
                    let list = fe.object_dropper.quick_shown && fe.object_dropper.feed.quick.as_ref().is_some_and(|q| q.page == PAGE_STYLE || q.page == PAGE_BRANDING);
                    if let Some(quick_id) = self.object_quick_menu_id.filter(|_| list) {
                        update_input(stage, quick_id, key, controller, &mut fe)?;
                        continue;
                    }
                }
                let target = stack.input_target();
                let Some(screen) = target.filter(|t| [pause, options, cal, trick, map, extras, viewer, profile, spawn, free, browser, park_setup, cac, team].contains(&Some(*t))) else { continue };
                // sub_825EC0E8: the screen's C++ handler first, then the movie's UpdateInput when the handler did
                // not consume the key and the screen's record says so (+12).
                let mut consumed = false;
                let movie_id = if Some(screen) == pause {
                    crossbar_id
                } else if Some(screen) == options {
                    options_id
                } else if Some(screen) == cal {
                    cal_id
                } else if Some(screen) == map {
                    map_id
                } else if Some(screen) == extras {
                    extras_id
                } else if Some(screen) == viewer {
                    viewer_id
                } else if Some(screen) == profile {
                    profile_id
                } else if Some(screen) == spawn {
                    spawn_id
                } else if Some(screen) == free {
                    free_id
                } else if Some(screen) == browser {
                    browser_id
                } else if Some(screen) == park_setup {
                    park_setup_id
                } else if Some(screen) == cac {
                    cac_id
                } else if Some(screen) == team {
                    team_id
                } else {
                    trick_id
                };
                if Some(screen) == pause && self.fade.held(FADE_SCREEN) {
                    // The crossbar's closing flags (+380 / +52) are set while the fade runs: no input.
                    continue;
                }
                if Some(screen) == trick && (self.fade.held(FADE_SCREEN) || self.trick_guide_fade_in.is_some()) {
                    // Input waits for state 2 (0x82619A20 releases the fade first; vt+40 needs S+192 == 2) and stops
                    // while the close fades out (S+52).
                    continue;
                }
                if Some(screen) == browser {
                    // The browser's C++ keys (82621778): the d-pad, the tab shoulders, B; A / Y / X go to the movie
                    // (its UpdateInput) until it is shown (no input while it waits for data).
                    use crate::fe::browser_host::BrowserKey;
                    let k = match key {
                        APT_PREVIOUS => Some(BrowserKey::Back),
                        KEY_RB => Some(BrowserKey::TopNext),
                        KEY_LB => Some(BrowserKey::TopPrevious),
                        KEY_RT => Some(BrowserKey::BottomNext),
                        KEY_LT => Some(BrowserKey::BottomPrevious),
                        // Only the pad's own codes (0xC - 0xF): a d-pad press also sends its AptD* twin.
                        k if k == KEYS_UP[0] => Some(BrowserKey::Up),
                        k if k == KEYS_DOWN[0] => Some(BrowserKey::Down),
                        k if k == KEYS_LEFT[0] => Some(BrowserKey::Left),
                        k if k == KEYS_RIGHT[0] => Some(BrowserKey::Right),
                        _ => None,
                    };
                    match k {
                        Some(k) => {
                            consumed = fe.browser.key(k);
                            if std::mem::take(&mut fe.browser.closed) {
                                stack.request(Layer::Main, Request::Close { screen });
                            }
                        }
                        None if !fe.browser.shown => consumed = true,
                        None => {}
                    }
                } else if Some(screen) == cac {
                    // FE 15's C++ keys (825FC7E8): Left, Right and Back; the movie moves the selection and takes A.
                    use skate_core::cac::Key;
                    let k = match key {
                        APT_PREVIOUS => Some(Key::Back),
                        // The placement pages' own: A goes back to Pete's, X starts over, Y puts the top on and off.
                        APT_NEXT => Some(Key::Accept),
                        APT_OPTION => Some(Key::Reset),
                        APT_MISC => Some(Key::Toggle),
                        // Only the pad's own codes (0xE, 0xF, as 825FC7E8): a d-pad press also sends its AptD* twin.
                        k if k == KEYS_LEFT[0] => Some(Key::Left),
                        k if k == KEYS_RIGHT[0] => Some(Key::Right),
                        _ => None,
                    };
                    if !fe.cac.shown {
                        consumed = true;
                    } else if let (Some(k), Some(c)) = (k, fe.cac.screen.as_mut()) {
                        // Retail's handler returns whether it took the key; the movie gets the others.
                        consumed = c.key(k);
                        sounds.extend(c.sounds.drain(..).map(String::from));
                        if std::mem::take(&mut c.closed) {
                            stack.request(Layer::Main, Request::Close { screen });
                        }
                    }
                } else if Some(screen) == team {
                    // FE 67's C++ keys (82651EE0): B, Y and X; the movie moves the cards and the options and takes A.
                    use skate_core::team::Key;
                    let k = match key {
                        APT_PREVIOUS => Some(Key::Back),
                        APT_OPTION => Some(Key::Logos),
                        APT_MISC => Some(Key::TeamName),
                        _ => None,
                    };
                    if !fe.team.shown {
                        consumed = true;
                    } else if let (Some(k), Some(t)) = (k, fe.team.screen.as_mut()) {
                        consumed = t.key(k);
                        sounds.extend(t.sounds.drain(..).map(String::from));
                        if std::mem::take(&mut t.closed) {
                            fe.team.leaving = true;
                            stack.request(Layer::Main, Request::Close { screen });
                        }
                    }
                } else if Some(screen) == park_setup {
                    // Park Setup's C++ keys (825D28C0): the movie takes none.
                    use crate::fe::park_setup_host::ParkSetupKey;
                    let k = match key {
                        APT_PREVIOUS => Some(ParkSetupKey::Back),
                        APT_NEXT => Some(ParkSetupKey::Select),
                        // Only the pad's own codes (0xC - 0xF, as 825D28C0): a d-pad press also sends its AptD* twin.
                        k if k == KEYS_UP[0] => Some(ParkSetupKey::Up),
                        k if k == KEYS_DOWN[0] => Some(ParkSetupKey::Down),
                        k if k == KEYS_LEFT[0] => Some(ParkSetupKey::Left),
                        k if k == KEYS_RIGHT[0] => Some(ParkSetupKey::Right),
                        _ => None,
                    };
                    if let Some(k) = k.filter(|_| fe.park_setup.shown) {
                        fe.park_setup.key(k);
                        if std::mem::take(&mut fe.park_setup.closed) {
                            stack.request(Layer::Main, Request::Close { screen });
                        }
                    }
                    consumed = true;
                } else if Some(screen) == free {
                    // Screen 34 input 8260A4E8 (b564): Up / Down pick the row (sound, SetState 999), Left / Right step
                    // it (SetState row; the movie plays the slider sound), B writes back and closes; the movie takes no
                    // keys.
                    use skate_core::free_skate_options::Key;
                    let k = match key {
                        APT_PREVIOUS => Some(Key::Back),
                        k if KEYS_UP.contains(&k) => Some(Key::Up),
                        k if KEYS_DOWN.contains(&k) => Some(Key::Down),
                        k if KEYS_LEFT.contains(&k) => Some(Key::Left),
                        k if KEYS_RIGHT.contains(&k) => Some(Key::Right),
                        _ => None,
                    };
                    consumed = true;
                    if let Some(k) = k {
                        let f = &mut fe.free_skate;
                        let out = f.screen.key(k, &f.rules);
                        sounds.extend(out.sound);
                        f.states.extend(out.states);
                        if let Some(values) = out.close {
                            // Flow 0x8B: the values go to the session, then the screen closes.
                            log.push(format!("free play options {values:?}"));
                            *free_play_out = Some(values);
                            stack.request(Layer::Main, Request::Close { screen });
                        }
                    }
                } else if Some(screen) == spawn {
                    // Screen 62 input 0x8264DF18 (b543): B closes (not in the demo mode), X dismisses a called skater or
                    // opens the Free Play options; never consumed (NOT RETAIL YET: the return value is not read), so the
                    // movie's picker also gets every key. Y (gamercard) is console platform UI.
                    let s = &mut fe.spawn_skater;
                    let out = match key {
                        APT_PREVIOUS => s.screen.back(&s.rules),
                        APT_MISC => s.screen.x(&s.rules),
                        _ => Default::default(),
                    };
                    s.take(out);
                } else if Some(screen) == profile {
                    // Screen 61 input 0x8264C608 (b308): Back closes, the tab keys step the tab, Up / Down scroll in
                    // the movie; Y (gamercard) and X (achievements) are console platform UI with no equivalent.
                    use skate_core::offline_profile::ProfileKey;
                    let key = match key {
                        APT_PREVIOUS => Some(ProfileKey::Back),
                        PROFILE_NEXT_TAB => Some(ProfileKey::NextTab),
                        PROFILE_PREVIOUS_TAB => Some(ProfileKey::PreviousTab),
                        k if PROFILE_SCROLL.contains(&k) => Some(ProfileKey::Scroll),
                        _ => None,
                    };
                    if let Some(k) = key.filter(|_| fe.offline_profile.loaded) {
                        let p = &mut fe.offline_profile;
                        let out = p.screen.key(k, &p.rules);
                        consumed = out.consumed;
                        if let Some(sound) = &out.sound {
                            sounds.push(sound.clone());
                        }
                        p.states.extend(out.states);
                        if out.close {
                            stack.request(Layer::Main, Request::Close { screen });
                        }
                    }
                } else if Some(screen) == viewer {
                    // Screen 3 input 0x8263A5A8 (b322), view mode: Back plays core_b_button and closes; the movie scrolls
                    // with the left stick (not consumed).
                    if key == APT_PREVIOUS {
                        sounds.push("core_b_button".into());
                        stack.request(Layer::Main, Request::Close { screen });
                    }
                } else if Some(screen) == extras {
                    // Screen 18 input 0x82655658 (b320): Back plays core_b_button and leaves the Skate Reel list
                    // (SetState(10)) or pops the screen; never consumed, so every key also reaches the movie's picker.
                    if key == APT_PREVIOUS {
                        let x = &mut fe.extras;
                        let out = x.screen.back(&x.rules);
                        sounds.push(out.sound.clone());
                        match out.state {
                            Some(state) => x.states.push(state),
                            None => stack.request(Layer::Main, Request::Close { screen }),
                        }
                    }
                } else if Some(screen) == map {
                    // Screen 17 input 82646B28 (b256): B hides the legend, pops a folder or closes at the root; the tab
                    // keys switch tabs (clamped). Up / down / A stay with the movie's picker (OnMenuHilight /
                    // OnMenuSelect). NOT RETAIL YET: Y (legend) and X (online difficulty) are not mapped.
                    let cm = &mut fe.challenge_map;
                    if key == APT_PREVIOUS {
                        consumed = true;
                        sounds.push("core_b_button".into());
                        let action = cm.state.back(fe.registry, fe.context);
                        cm.apply(action);
                    } else if MAP_NEXT_TAB.contains(&key) || MAP_PREVIOUS_TAB.contains(&key) {
                        consumed = true;
                        let action = cm.state.step_tab(fe.registry, fe.context, MAP_NEXT_TAB.contains(&key));
                        if action != skate_core::challenge_map::Action::None {
                            sounds.push("core_tab".into());
                        }
                        cm.apply(action);
                    }
                } else if Some(screen) == trick {
                    // Screen 14 vt+40 (0x82618F38): Back closes at level 0 or goes up a level (SetState(0)); A on a row
                    // without a list is consumed. Back plays core_b_button (h64 0x4D927CD64CC8B818).
                    let tg = &mut fe.trick_guide;
                    match key {
                        APT_PREVIOUS => {
                            sounds.push("core_b_button".into());
                            match tg.state.back() {
                                // Level 0: hold the fade; the screen closes when black (callback 0x82618EF0).
                                skate_core::trick_guide::Back::Close => {
                                    let params = self.fade_params;
                                    if let Some(action) = self.fade.hold(FADE_SCREEN, Some(FadeAction::Close(screen)), false, &params) {
                                        run_fade_action(stack, action);
                                    }
                                }
                                skate_core::trick_guide::Back::Up(state) => tg.states.push(state),
                            }
                        }
                        APT_NEXT => consumed = tg.state.consumes_select(&tg.guide),
                        _ => {}
                    }
                } else if Some(screen) == cal {
                    // Screen 51 input 0x8260E890: A advances (0x8260E950), B goes back a page or closes; both play
                    // their sound and leave the key to the movie too (the handler returns 0).
                    let rules = &self.video_cal_rules;
                    let action = match key {
                        APT_NEXT => {
                            sounds.push(rules.sound_next.clone());
                            Some(self.video_cal.advance(rules))
                        }
                        APT_PREVIOUS => {
                            sounds.push(rules.sound_back.clone());
                            Some(self.video_cal.back())
                        }
                        _ => None,
                    };
                    if let Some(action) = action {
                        run_calibration(stage, stack, &mut self.popups, &mut self.popup_dirty, rules, cal_id, action, screen, &mut fe, log)?;
                    }
                } else if Some(screen) == pause {
                    // Crossbar sub_8261B560: consumes only its close branch.
                    if let Some(event) = NavEvent::from_key_index(key) {
                        let nav = fe.state.handle(event, &fe.view, &self.rules, false);
                        if let Some(sound) = &nav.sound {
                            sounds.push(sound.clone());
                        }
                        if let Some(state) = nav.set_state {
                            // Retail passes both as "%d" strings (sub_8263F320 index 10).
                            set_state(stage, crossbar_id, state, &mut fe)?;
                        }
                        match nav.action {
                            Some(CrossbarAction::Close) => {
                                consumed = true;
                                if self.closable {
                                    stack.request(Layer::Main, Request::Close { screen });
                                }
                            }
                            Some(CrossbarAction::SelectSubOption { tab, row, sub }) => {
                                sub_selected = fe.view.item(tab, row).map(|item| (item.to_owned(), sub));
                            }
                            Some(CrossbarAction::SelectItem { tab, row }) => {
                                selected = fe.view.item(tab, row).map(str::to_owned);
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Settings sub_8260C258: Back (3) leaves a sub-screen for the main list (sub_8260D078) or closes
                    // the screen; Left / Right (14 / 15) step the highlighted row's value, then SetState(50)
                    // refreshes. NOT RETAIL YET: the per-row UI sounds are fe records not named yet; nav left / right
                    // stand in.
                    match key {
                        APT_PREVIOUS => {
                            consumed = true;
                            sounds.push("core_b_button".into());
                            if fe.settings.screen > 1 {
                                log.push(format!("settings back to the main list from screen {}", fe.settings.screen));
                                fe.settings.screen = 1;
                                fe.settings.row = 0;
                                set_state(stage, options_id, 1, &mut fe)?;
                            } else {
                                stack.request(Layer::Main, Request::Close { screen });
                            }
                        }
                        APT_LEFT | APT_RIGHT => {
                            consumed = true;
                            if let Some(id) = fe.settings.row_id(fe.settings.row) {
                                let left = key == APT_LEFT;
                                steps.push((id.to_owned(), if left { Direction::Left } else { Direction::Right }));
                                sounds.push(if left { "core_nav_left" } else { "core_nav_right" }.into());
                            }
                        }
                        _ => {}
                    }
                }
                if !consumed && info(screens, screen).input_to_movie {
                    update_input(stage, movie_id, key, controller, &mut fe)?;
                }
                flush_trick_guide(stage, trick_id, &mut fe)?;
                flush_challenge_map(stage, stack, map, map_id, &mut fe)?;
                flush_offline_profile(stage, profile_id, &mut fe)?;
            }
            drop(keys_span);
            let effects_span = crate::fe::profile::span(crate::fe::profile::Span::Effects);
            // The stack's update (sub_82D0A560) and what it asks of the movies and the game.
            let effects = drop_reopen_and_close(stack.update(&|i| info(screens, i)));
            if !effects.is_empty() {
                log.push(format!("APT objects {} before {} screen change(s)", stage.object_summary(), effects.len()));
            }
            for effect in effects {
                match effect {
                    Effect::Open(i) if Some(i) == pause => {
                        log.push(format!("open pause menu (FE {i})"));
                        // Crossbar init (vt slot 3, sub_8261ACA8) releases the screen fade.
                        self.fade.release(FADE_SCREEN, false, &self.fade_params);
                        fe.state = CrossbarState::open(&fe.view);
                        open_screen(stage, CROSSBAR_PATH, CROSSBAR, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == options => {
                        // Settings init sub_8260BF58: screen index -1 -> 0 (the pause path opens on the main list).
                        log.push(format!("open Game Settings (FE {i})"));
                        fe.settings.screen = 0;
                        fe.settings.row = 0;
                        open_screen(stage, OPTIONS_PATH, OPTIONS, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == trick => {
                        log.push(format!("open Trick Guide (FE {i})"));
                        fe.trick_guide.reset();
                        // Its callbacks are ours from the load on (the movie lists its rows while loading).
                        fe.trick_guide.open = true;
                        open_screen(stage, TRICK_GUIDE_PATH, TRICK_GUIDE, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == trick => {
                        log.push(format!("close Trick Guide (FE {i})"));
                        fe.trick_guide.open = false;
                        self.trick_guide_fade_in = None;
                        close(stage, trick_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == browser => {
                        log.push(format!("open browser (FE {i})"));
                        fe.browser.open = true;
                        fe.browser.shown = false;
                        open_screen(stage, BROWSER_PATH, BROWSER, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == browser => {
                        log.push(format!("close browser (FE {i})"));
                        fe.browser.open = false;
                        fe.browser.shown = false;
                        close(stage, browser_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == team => {
                        log.push(format!("open the team screen (FE {i})"));
                        fe.team.shown = false;
                        open_screen(stage, TEAM_PATH, TEAM, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == team => {
                        log.push(format!("close the team screen (FE {i})"));
                        fe.team.shown = false;
                        if std::mem::take(&mut fe.team.leaving) {
                            fe.team.screen = None;
                        }
                        close(stage, team_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == cac => {
                        log.push(format!("open the customisation (FE {i})"));
                        fe.cac.shown = false;
                        open_screen(stage, CAC_PATH, CAC, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == cac => {
                        log.push(format!("close the customisation (FE {i})"));
                        fe.cac.shown = false;
                        fe.cac.finished = fe.cac.screen.take().map(|c| c.record);
                        close(stage, cac_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == park_setup => {
                        log.push(format!("open park setup (FE {i})"));
                        fe.park_setup.open = true;
                        fe.park_setup.shown = false;
                        open_screen(stage, PARK_SETUP_PATH, PARK_SETUP, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == park_setup => {
                        log.push(format!("close park setup (FE {i})"));
                        fe.park_setup.open = false;
                        fe.park_setup.shown = false;
                        close(stage, park_setup_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == map => {
                        // Init sub_826457E0 on the mode root (sub_8261BC30: `progression` in career and free skate).
                        log.push(format!("open Challenge Map (FE {i})"));
                        fe.challenge_map.open(fe.registry, fe.context, "progression");
                        open_screen(stage, CHALLENGE_MAP_PATH, CHALLENGE_MAP, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == map => {
                        log.push(format!("close Challenge Map (FE {i})"));
                        fe.challenge_map.open = false;
                        close(stage, map_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == profile => {
                        log.push(format!("open Skate Profile (FE {i})"));
                        fe.offline_profile.open();
                        open_screen(stage, OFFLINE_PROFILE_PATH, OFFLINE_PROFILE, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == profile => {
                        log.push(format!("close Skate Profile (FE {i})"));
                        fe.offline_profile.loaded = false;
                        fe.offline_profile.states.clear();
                        close(stage, profile_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == spawn => {
                        let groups = crate::fe::spawn_skater_host::groups(&self.callable);
                        let called = self.called.iter().map(|k| skate_core::spawn_skater::Skater::Character(k.clone())).collect();
                        let s = &mut fe.spawn_skater;
                        s.screen = skate_core::spawn_skater::SpawnSkater::open(self.spawn_skater_mode, &groups, called);
                        log.push(format!("open SpawnSkater (FE {i}) mode {} rows {}", s.screen.mode.code(), s.screen.rows().len()));
                        s.states.clear();
                        s.actions.clear();
                        s.open = true;
                        open_screen(stage, SPAWN_SKATER_PATH, SPAWN_SKATER, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == spawn => {
                        log.push(format!("close SpawnSkater (FE {i})"));
                        fe.spawn_skater.open = false;
                        close(stage, spawn_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == free => {
                        // Init copies the session's values into the screen (b564).
                        log.push(format!("open Free Play options (FE {i}) {:?}", self.free_play));
                        fe.free_skate.screen = skate_core::free_skate_options::FreeSkateOptions::open(self.free_play);
                        fe.free_skate.states.clear();
                        fe.free_skate.open = true;
                        open_screen(stage, FREE_SKATE_PATH, FREE_SKATE, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == loader => {
                        log.push(format!("open loading screen (FE {i})"));
                        fe.tips_loader.open = true;
                        fe.tips_loader.tips.open();
                        // The Stats panel's rows (offline fetch sub_82607840); none = no Stats panel.
                        let mut seed = fe.tips_loader.seed;
                        let numbers = crate::fe::fe_language::NumberFormat::default();
                        fe.tips_loader.stats = crate::fe::tips_loader_host::stat_rows(&self.career_stats, &self.career_table, fe.language, &numbers, self.imperial, &mut || crate::fe::tips_loader_host::lcg(&mut seed));
                        fe.tips_loader.seed = seed;
                        self.load_up = false;
                        self.load_gone = false;
                        open_screen(stage, TIPS_LOADER_PATH, TIPS_LOADER, &mut fe)?;
                        load_tag(stage, true, &mut fe, log);
                    }
                    Effect::Close(i) if Some(i) == loader => {
                        log.push(format!("close loading screen (FE {i})"));
                        fe.tips_loader.open = false;
                        self.load_up = false;
                        self.load_gone = true;
                        close(stage, loader_id, &mut fe)?;
                        load_tag(stage, false, &mut fe, log);
                    }
                    Effect::Close(i) if Some(i) == free => {
                        log.push(format!("close Free Play options (FE {i})"));
                        fe.free_skate.open = false;
                        close(stage, free_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == extras => {
                        log.push(format!("open Extras (FE {i})"));
                        fe.extras.screen = skate_core::extras::Extras::open();
                        fe.extras.states.clear();
                        fe.extras.open = true;
                        open_screen(stage, EXTRAS_PATH, EXTRAS, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == extras => {
                        log.push(format!("close Extras (FE {i})"));
                        fe.extras.open = false;
                        close(stage, extras_id, &mut fe)?;
                    }
                    Effect::Open(i) if Some(i) == viewer => {
                        log.push(format!("open text viewer (FE {i})"));
                        open_screen(stage, TEXT_VIEWER_PATH, TEXT_VIEWER, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == viewer => {
                        log.push(format!("close text viewer (FE {i})"));
                        close(stage, viewer_id, &mut fe)?;
                        fe.tos.viewer = None;
                    }
                    // FE 46 (b305): no movie; the game shows the Vita's system keyboard while it is on top.
                    Effect::Open(i) if Some(i) == entry => {
                        if let Some(request) = self.pending_entry.take() {
                            log.push(format!("open text entry (FE {i}) for {}", request.callback));
                            self.entry = Some(request);
                            self.entry_ask = true;
                            self.entry_answer = None;
                        }
                        stack.ready(i);
                    }
                    Effect::Close(i) if Some(i) == entry => {
                        log.push(format!("close text entry (FE {i})"));
                        self.entry = None;
                        self.entry_ask = false;
                    }
                    Effect::Open(i) if Some(i) == cal => {
                        log.push(format!("open Video Calibration (FE {i})"));
                        self.video_cal = VideoCalibration::open();
                        open_screen(stage, VIDEO_CALIBRATION_PATH, VIDEO_CALIBRATION, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == cal => {
                        log.push(format!("close Video Calibration (FE {i})"));
                        close(stage, cal_id, &mut fe)?;
                    }
                    // Screen vt+28 -> facade ShowScreen (0x825DA410): the AS id as "%d" and "true" / "false".
                    Effect::Show(i, visible) if [pause, options, cal, trick, map, extras, profile, spawn, free].contains(&Some(i)) => {
                        let id = [(pause, crossbar_id), (options, options_id), (cal, cal_id), (trick, trick_id), (map, map_id), (extras, extras_id), (profile, profile_id), (spawn, spawn_id), (free, free_id)]
                            .into_iter()
                            .find(|(s, _)| *s == Some(i))
                            .map_or(0, |(_, id)| id);
                        show_screen(stage, id, visible, &mut fe)?;
                    }
                    Effect::Close(i) if Some(i) == pause => {
                        log.push(format!("close pause menu (FE {i})"));
                        close(stage, crossbar_id, &mut fe)?
                    }
                    Effect::Close(i) if Some(i) == options => {
                        log.push(format!("close Game Settings (FE {i})"));
                        // Begin close sub_8260C048: SetScreenState(2) outside the front end (FE state 2), then the
                        // outro.
                        set_state(stage, options_id, 2, &mut fe)?;
                        close(stage, options_id, &mut fe)?;
                        fe.settings.open = false;
                    }
                    Effect::Open(i) | Effect::Close(i) => log.push(format!("FE screen {i} has no template yet")),
                    Effect::SuspendGameplay(on) => {
                        log.push(format!("gameplay {}", if on { "suspended" } else { "resumed" }));
                        self.suspended = on;
                    }
                    // vt+44(hud, online): 0 both on, 1 both off, 2 online only (3 = keep is never emitted).
                    Effect::Hud(mode) => {
                        self.hud = match mode {
                            HudMode::On | HudMode::Keep => FeHud { hud: true, online: true },
                            HudMode::Off => FeHud { hud: false, online: false },
                            HudMode::OnlineOnly => FeHud { hud: false, online: true },
                        };
                    }
                    // vt+48 -> sub_825E4408: on a change, the chyron movie's _global.Chyron_SetVisible("true" /
                    // "false"). NOT RETAIL YET: retail plays Chyron_PlayOutro instead of hiding when the chyron is
                    // mid-show.
                    Effect::Chyron(on) if on != self.chyron => {
                        self.chyron = on;
                        let defined = matches!(stage.vm.get(stage.vm.global, "Chyron_SetVisible"), Value::Object(_));
                        if defined {
                            stage.call_path("_global.Chyron_SetVisible", vec![Value::Text(if on { "true" } else { "false" }.into())], &mut fe)?;
                        }
                    }
                    Effect::Chyron(_) => {}
                    Effect::Blur(_) | Effect::Show(..) | Effect::Param(..) | Effect::Focus(_) => {}
                }
            }
            drop(effects_span);
            // The instant replay screen (FE 12 in instant mode): opened with the replay, shown after its IntroComplete
            // (b391: retail never calls the movie's InstantDone; the screen manager's ShowScreen makes it visible), fed
            // every frame and told what changed; closed with the replay (its help first).
            if let Some(replay_id) = replay_id {
                match (replay_feed.clone().filter(|f| f.active), fe.replay_menu.open) {
                    (Some(feed), false) => {
                        log.push(format!("open {REPLAY} (FE 12, instant replay)"));
                        fe.replay_menu.reset();
                        fe.replay_menu.open = true;
                        fe.replay_menu.feed = feed;
                        *replay_shown = false;
                        open_screen(stage, REPLAY_PATH, REPLAY, &mut fe)?;
                    }
                    (Some(feed), true) => fe.replay_menu.feed = feed,
                    (None, true) => {
                        log.push(format!("close {REPLAY}"));
                        if let Some(help_id) = replay_help_id.filter(|_| fe.replay_menu.help_open) {
                            close(stage, help_id, &mut fe)?;
                        }
                        fe.replay_menu.open = false;
                        fe.replay_menu.help_open = false;
                        fe.replay_menu.help_shown = false;
                        *replay_shown = false;
                        close(stage, replay_id, &mut fe)?;
                    }
                    (None, false) => {}
                }
            }
            // The Object Dropper screen (FE 0x35 / 0x37): opened with the editor, shown after its IntroComplete, fed
            // every frame, closed with the editor.
            if let Some(dropper_id) = object_dropper_id {
                // A screen still playing its outro is let finish first (opened over it, the new one never shows).
                let closing = stage.levels().iter().any(|l| l.key == movie_key(OBJECT_DROPPER_PATH));
                match (object_dropper_feed.clone(), fe.object_dropper.open) {
                    (Some(_), false) if closing => {}
                    (Some(feed), false) => {
                        log.push(format!("open {OBJECT_DROPPER} ({})", if feed.world { "FE 0x37, open world" } else { "FE 0x35, park" }));
                        // Modes posted while a closing screen was let finish come along.
                        let states = std::mem::take(&mut fe.object_dropper.states);
                        fe.object_dropper = crate::fe::object_dropper_host::ObjectDropperHost { open: true, feed, states, ..Default::default() };
                        open_screen(stage, OBJECT_DROPPER_PATH, OBJECT_DROPPER, &mut fe)?;
                    }
                    (Some(feed), true) => fe.object_dropper.feed = feed,
                    (None, true) => {
                        log.push(format!("close {OBJECT_DROPPER}"));
                        fe.object_dropper = Default::default();
                        close(stage, dropper_id, &mut fe)?;
                    }
                    (None, false) => {}
                }
                if object_dropper_feed.is_some() {
                    fe.object_dropper.states.extend(object_dropper_posts.iter().map(|&m| i32::from(m)));
                }
                // The Quick Menu (FE 0x36) over the shown dropper screen while the editor's is open.
                if let Some(quick_id) = object_quick_menu_id {
                    let wanted = object_dropper_feed.as_ref().is_some_and(|f| f.quick.is_some()) && fe.object_dropper.shown;
                    let host = &mut fe.object_dropper;
                    match (wanted, host.quick_open) {
                        (true, false) => {
                            log.push(format!("open {OBJECT_QUICK_MENU} (FE 0x36)"));
                            host.quick_open = true;
                            host.quick_shown = false;
                            host.quick_states.clear();
                            open_screen(stage, OBJECT_QUICK_MENU_PATH, OBJECT_QUICK_MENU, &mut fe)?;
                        }
                        (false, true) => {
                            log.push(format!("close {OBJECT_QUICK_MENU}"));
                            host.quick_open = false;
                            host.quick_shown = false;
                            close(stage, quick_id, &mut fe)?;
                        }
                        _ => {}
                    }
                    if fe.object_dropper.quick_open {
                        fe.object_dropper.quick_states.extend(object_quick_posts.iter().map(|&m| i32::from(m)));
                    }
                }
            }
            // NOT RETAIL (guard): with no screen left the fade would stay black, so release it.
            if stack.top(Layer::Main).is_none() && self.fade.held(FADE_SCREEN) {
                self.fade.release(FADE_SCREEN, false, &self.fade_params);
            }
            // The fader's ticks (FE cadence): actions due at black.
            let params = self.fade_params;
            let period = 1.0 / params.tick_hz.max(1.0);
            let ticks = ((seconds + self.fade_carry) / period).floor() as u32;
            self.fade_carry = (seconds + self.fade_carry - ticks as f32 * period).min(period * 30.0);
            for _ in 0..ticks.min(30) {
                // The Trick Guide's state-4 count, then actions due at black.
                if let Some(n) = self.trick_guide_fade_in {
                    self.trick_guide_fade_in = n.checked_sub(1).filter(|n| *n > 0);
                    if self.trick_guide_fade_in.is_none() {
                        self.fade.release(FADE_SCREEN, false, &params);
                    }
                }
                for action in self.fade.tick() {
                    run_fade_action(stack, action);
                }
            }
            // The loading screen around a world load (game state 0x82707508; doc 31 "Loading screen"): fade to black,
            // push FE 52, drop the black when it is up; when the load is done, black, close, a few black updates, fade in.
            if let Some(l) = loader {
                use skate_core::tips_loader::LoadStep;
                let step = self.load.update(self.loading, self.fade.alpha >= 1.0, self.load_up, self.load_gone);
                match step {
                    Some(LoadStep::HoldBlack) => {
                        self.fade.hold(FADE_LOAD, None, false, &params);
                    }
                    Some(LoadStep::Push) => stack.request(Layer::Main, Request::push(l)),
                    Some(LoadStep::ReleaseInstant) => self.fade.release(FADE_LOAD, true, &params),
                    Some(LoadStep::Close) => stack.request(Layer::Main, Request::Close { screen: l }),
                    Some(LoadStep::Release) => {
                        self.load_gone = false;
                        self.fade.release(FADE_LOAD, false, &params);
                    }
                    None => {}
                }
                if let Some(step) = step {
                    log.push(format!("loading screen {step:?}"));
                }
            }
            if self.save_tag != self.save_tag_shown {
                save_tag(stage, self.save_tag, &mut fe, log);
                self.save_tag_shown = self.save_tag;
            }
            stage.update(seconds * 1000.0, &movies.movies, &mut fe)?;
            // The instant replay screen after this frame's movie update: shown once its intro completed, then told what
            // changed in the replay; its help (FE 13) opened and closed with the feed, each page turn posted.
            // The browser: shown after its intro (ready: keys), then its modes in order.
            if let Some(b) = browser.filter(|_| fe.browser.open) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({browser_id})"));
                if !fe.browser.shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({browser_id})")) {
                    fe.browser.shown = true;
                    stack.ready(b);
                    show_screen(stage, browser_id, true, &mut fe)?;
                }
                if fe.browser.shown {
                    for state in std::mem::take(&mut fe.browser.states) {
                        set_state(stage, browser_id, state, &mut fe)?;
                    }
                }
            }
            // The customisation: shown after its intro (ready: keys), then the movie's states and the row sounds.
            if let Some(c) = cac.filter(|_| fe.cac.screen.is_some()) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({cac_id})"));
                if !fe.cac.shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({cac_id})")) {
                    fe.cac.shown = true;
                    stack.ready(c);
                    show_screen(stage, cac_id, true, &mut fe)?;
                }
                if fe.cac.shown {
                    let states = fe.cac.screen.as_mut().map(|s| std::mem::take(&mut s.states)).unwrap_or_default();
                    for state in states {
                        set_state(stage, cac_id, state, &mut fe)?;
                    }
                }
                if let Some(s) = fe.cac.screen.as_mut().filter(|_| fe.cac.shown) {
                    if !self.popups.shown().is_some() {
                        s.hold(self.cac_axis, seconds);
                        s.turn_by(self.cac_turn, seconds);
                        s.adjust_by(self.cac_adjust, seconds);
                        s.tick(seconds);
                    }
                    if let Some(lower) = s.ask_tattoo.take() {
                        self.popups.push(skate_core::fe_popup::PopupRequest {
                            title: "ID_COMMON_WARNING_TITLE".into(),
                            description: if lower { "ID_CAC_LOWER_TATTOO_CHANGE_CONFIRMATION" } else { "ID_CAC_UPPER_TATTOO_CHANGE_CONFIRMATION" }.into(),
                            options: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
                            callback: if lower { CAC_TATTOO_LOWER_POPUP } else { CAC_TATTOO_UPPER_POPUP }.into(),
                            ..Default::default()
                        });
                        self.popup_dirty = true;
                    }
                    if std::mem::take(&mut s.ask_exit) {
                        self.popups.push(skate_core::fe_popup::PopupRequest {
                            title: "ID_COMMON_WARNING_TITLE".into(),
                            description: "ID_CAC_EXIT_CAC_CONFIRMATION".into(),
                            options: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
                            callback: CAC_EXIT_POPUP.into(),
                            ..Default::default()
                        });
                        self.popup_dirty = true;
                    }
                    if std::mem::take(&mut s.ask_gender) {
                        self.popups.push(skate_core::fe_popup::PopupRequest {
                            title: "ID_COMMON_WARNING_TITLE".into(),
                            description: "ID_CAC_GENDERSWITCH_CONFIRMATION".into(),
                            options: vec!["ID_COMMON_YES".into(), "ID_COMMON_NO".into()],
                            callback: CAC_GENDER_POPUP.into(),
                            ..Default::default()
                        });
                        self.popup_dirty = true;
                    }
                }
                if let Some(s) = fe.cac.screen.as_mut() {
                    sounds.extend(s.sounds.drain(..).map(String::from));
                }
            }
            // The team screen: shown after its intro (82651A48), then its states; the empty-name warning is the game's
            // popup, the other asks go to the game.
            if let Some(t) = team.filter(|_| fe.team.screen.is_some()) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({team_id})"));
                if !fe.team.shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({team_id})")) {
                    fe.team.shown = true;
                    stack.ready(t);
                    show_screen(stage, team_id, true, &mut fe)?;
                    if let Some(s) = fe.team.screen.as_mut() {
                        s.shown();
                    }
                }
                let (states, effects, team_sounds) = match fe.team.screen.as_mut().filter(|_| fe.team.shown) {
                    Some(s) => (std::mem::take(&mut s.states), std::mem::take(&mut s.effects), std::mem::take(&mut s.sounds)),
                    None => Default::default(),
                };
                for state in states {
                    set_state(stage, team_id, state, &mut fe)?;
                }
                sounds.extend(team_sounds.into_iter().map(String::from));
                {
                    for effect in effects {
                        if effect == skate_core::team::Effect::EmptyTeamName {
                            self.popups.push(skate_core::fe_popup::PopupRequest {
                                title: "ID_COMMON_WARNING_TITLE".into(),
                                description: "ID_TEAM_MANAGEMENT_EMPTY_TEAM_NAME".into(),
                                options: vec!["ID_COMMON_OK".into()],
                                callback: TEAM_NAME_POPUP.into(),
                                ..Default::default()
                            });
                            self.popup_dirty = true;
                        } else {
                            team_effects.push(effect);
                        }
                    }
                }
            }
            // Park Setup: shown after its intro (ready: keys), then its refreshes.
            if let Some(p) = park_setup.filter(|_| fe.park_setup.open) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({park_setup_id})"));
                if !fe.park_setup.shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({park_setup_id})")) {
                    fe.park_setup.shown = true;
                    stack.ready(p);
                    show_screen(stage, park_setup_id, true, &mut fe)?;
                }
                if fe.park_setup.shown {
                    for state in std::mem::take(&mut fe.park_setup.states) {
                        set_state(stage, park_setup_id, state, &mut fe)?;
                    }
                }
            }
            // The Object Dropper screen: shown after its intro, then the editor's screen modes in order.
            if let Some(dropper_id) = object_dropper_id.filter(|_| fe.object_dropper.open) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({dropper_id})"));
                if !fe.object_dropper.shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({dropper_id})")) {
                    fe.object_dropper.shown = true;
                    show_screen(stage, dropper_id, true, &mut fe)?;
                }
                if let Some(quick_id) = object_quick_menu_id.filter(|_| fe.object_dropper.quick_open) {
                    take_call(&mut fe, &format!("Game.OnScreenLoaded({quick_id})"));
                    if !fe.object_dropper.quick_shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({quick_id})")) {
                        fe.object_dropper.quick_shown = true;
                        show_screen(stage, quick_id, true, &mut fe)?;
                    }
                    if fe.object_dropper.quick_shown {
                        for state in std::mem::take(&mut fe.object_dropper.quick_states) {
                            set_state(stage, quick_id, state, &mut fe)?;
                        }
                    }
                }
                if fe.object_dropper.shown {
                    for state in std::mem::take(&mut fe.object_dropper.states) {
                        // A catalogue state builds its lists from scratch (about 3,000 script objects): collected
                        // first when half the VM's budget is in use, else a few in one update pass the budget (the
                        // stage collects only after its update; device run 2026-10-29).
                        if stage.vm.live_objects() > crate::fe::apt_vm::MAX_OBJECTS / 2 {
                            stage.collect()?;
                        }
                        set_state(stage, dropper_id, state, &mut fe)?;
                    }
                }
            }
            if let Some(replay_id) = replay_id.filter(|_| fe.replay_menu.open) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({replay_id})"));
                if !*replay_shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({replay_id})")) {
                    *replay_shown = true;
                    show_screen(stage, replay_id, true, &mut fe)?;
                }
                if *replay_shown {
                    fe.replay_menu.post_changes();
                    for state in std::mem::take(&mut fe.replay_menu.states) {
                        fe.replay_menu.posting = Some(state);
                        let posted = set_state(stage, replay_id, state, &mut fe);
                        fe.replay_menu.posting = None;
                        posted?;
                    }
                }
                if let Some(help_id) = replay_help_id.filter(|_| *replay_shown) {
                    let host = &mut fe.replay_menu;
                    match (host.feed.help, host.help_open) {
                        (true, false) => {
                            log.push(format!("open {REPLAY_HELP} (FE 13)"));
                            host.help_open = true;
                            host.help_shown = false;
                            host.help_turns_posted = host.feed.help_turns;
                            open_screen(stage, REPLAY_HELP_PATH, REPLAY_HELP, &mut fe)?;
                        }
                        (false, true) => {
                            log.push(format!("close {REPLAY_HELP}"));
                            host.help_open = false;
                            host.help_shown = false;
                            close(stage, help_id, &mut fe)?;
                        }
                        _ => {}
                    }
                    if fe.replay_menu.help_open {
                        take_call(&mut fe, &format!("Game.OnScreenLoaded({help_id})"));
                        if !fe.replay_menu.help_shown && take_call(&mut fe, &format!("Game.OnScreenIntroComplete({help_id})")) {
                            fe.replay_menu.help_shown = true;
                            show_screen(stage, help_id, true, &mut fe)?;
                        }
                        while fe.replay_menu.help_shown && fe.replay_menu.help_turns_posted < fe.replay_menu.feed.help_turns {
                            fe.replay_menu.help_turns_posted += 1;
                            set_state(stage, help_id, crate::fe::replay_menu_host::HELP_NEXT_PAGE, &mut fe)?;
                        }
                    }
                }
            }
            // Settings loaded (vt+108 sub_8260C010): SetScreenState(1) builds the rows; intro complete = ready.
            if let Some(o) = options.filter(|o| stack.top(Layer::Main) == Some(*o)) {
                if take_call(&mut fe, &format!("Game.OnScreenLoaded({options_id})")) {
                    fe.settings.open = true;
                    set_state(stage, options_id, 1, &mut fe)?;
                }
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({options_id})")) {
                    stack.ready(o);
                }
            }
            // Popup choices (host vt+64 0x825EF698): the callback by id, then the list shows its next entry.
            for choice in std::mem::take(&mut fe.popup.choices) {
                let Some(event) = self.popups.choose(choice) else { continue };
                log.push(format!("popup {} choice {} ({})", event.id, event.choice, event.callback));
                self.popup_dirty = true;
                if event.callback == VIDEO_CALIBRATION_POPUP {
                    if let Some(c) = cal.filter(|c| stack.active(*c)) {
                        let action = self.video_cal.popup_result(event.choice as u32, &self.video_cal_rules);
                        run_calibration(stage, stack, &mut self.popups, &mut self.popup_dirty, &self.video_cal_rules, cal_id, action, c, &mut fe, log)?;
                    }
                } else if event.callback == TEAM_NAME_POPUP {
                    team_effects.push(skate_core::team::Effect::RenameTeam);
                } else if event.callback == CAC_GENDER_POPUP {
                    if let Some(c) = fe.cac.screen.as_mut().filter(|_| event.choice == 0) {
                        c.switch_gender();
                    }
                } else if event.callback == CAC_TATTOO_UPPER_POPUP || event.callback == CAC_TATTOO_LOWER_POPUP {
                    if let Some(c) = fe.cac.screen.as_mut() {
                        c.tattoo_answer(event.callback == CAC_TATTOO_LOWER_POPUP, event.choice == 0);
                    }
                } else if event.callback == CAC_EXIT_POPUP {
                    if let Some(c) = fe.cac.screen.as_mut() {
                        c.exit_answer(event.choice == 0);
                        if std::mem::take(&mut c.closed) {
                            if let Some(screen) = cac {
                                stack.request(Layer::Main, Request::Close { screen });
                            }
                        }
                    }
                } else {
                    popup_choices.push((event.callback, event.choice));
                }
            }
            if std::mem::take(&mut self.popup_dirty) {
                show_popup(stage, &self.popups, &self.popup_ids, &mut fe, sounds, log)?;
            }
            // The teleporter prompt: state 2 with the title when it appears or changes, 0 when it goes (upstream
            // `retail_menu_stage`).
            if let Some(signup) = self.signup_id {
                if self.signup_title != self.signup_shown {
                    fe.signup_title = self.signup_title.clone();
                    let state = if self.signup_title.is_some() { 2 } else { 0 };
                    set_state(stage, signup, state, &mut fe)?;
                    log.push(format!("signup state {state} title {:?}", self.signup_title));
                    self.signup_shown = self.signup_title.clone();
                }
            }
            // Video Calibration loaded (vt+108 0x8260E888): the first page; intro complete = ready.
            if let Some(c) = cal.filter(|c| stack.top(Layer::Main) == Some(*c)) {
                if take_call(&mut fe, &format!("Game.OnScreenLoaded({cal_id})")) {
                    let action = self.video_cal.advance(&self.video_cal_rules);
                    run_calibration(stage, stack, &mut self.popups, &mut self.popup_dirty, &self.video_cal_rules, cal_id, action, c, &mut fe, log)?;
                }
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({cal_id})")) {
                    stack.ready(c);
                }
            }
            // Trick Guide: the movie builds its list itself on load; intro complete = ready. The picker's callbacks
            // during the update queue screen states.
            if let Some(t) = trick.filter(|t| stack.top(Layer::Main) == Some(*t)) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({trick_id})"));
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({trick_id})")) {
                    stack.ready(t);
                    // State 4: count updates, then release the fade. NOT RETAIL YET: what enters state 4 is not found;
                    // the count starts at the intro's end.
                    self.trick_guide_fade_in = Some(TRICK_GUIDE_FADE_IN_TICKS);
                }
            }
            flush_trick_guide(stage, trick_id, &mut fe)?;
            // Challenge Map: intro complete = ready; its open states and the picker's callbacks go out after the update.
            if let Some(m) = map.filter(|m| stack.top(Layer::Main) == Some(*m)) {
                take_call(&mut fe, &format!("Game.OnScreenLoaded({map_id})"));
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({map_id})")) {
                    stack.ready(m);
                }
            }
            flush_challenge_map(stage, stack, map, map_id, &mut fe)?;
            // Skate Profile: init's page state goes out once the movie loaded; intro complete = ready.
            if let Some(p) = profile.filter(|p| stack.top(Layer::Main) == Some(*p)) {
                if take_call(&mut fe, &format!("Game.OnScreenLoaded({profile_id})")) {
                    fe.offline_profile.loaded = true;
                }
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({profile_id})")) {
                    stack.ready(p);
                }
            }
            flush_offline_profile(stage, profile_id, &mut fe)?;
            // Extras: loaded -> SetState(10) builds the list (NOT RETAIL YET: who posts retail's first 10 is not found,
            // b320); intro complete = ready. Then the rows the player picked.
            if let Some(x) = extras.filter(|x| stack.top(Layer::Main) == Some(*x)) {
                if take_call(&mut fe, &format!("Game.OnScreenLoaded({extras_id})")) {
                    fe.extras.states.push(fe.extras.rules.list_state);
                }
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({extras_id})")) {
                    stack.ready(x);
                }
            }
            // Text viewer: loaded hook 0x8263A2A0 posts SetState(1) (the movie splits nothing itself: InitWords);
            // intro complete = ready.
            if let Some(v) = viewer.filter(|v| stack.top(Layer::Main) == Some(*v)) {
                if take_call(&mut fe, &format!("Game.OnScreenLoaded({viewer_id})")) {
                    set_state(stage, viewer_id, 1, &mut fe)?;
                }
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({viewer_id})")) {
                    stack.ready(v);
                }
            }
            for state in std::mem::take(&mut fe.extras.states) {
                set_state(stage, extras_id, state, &mut fe)?;
            }
            // The loading screen: intro complete = up (the black goes at once); then a panel every 9.0 s (sub_82605CC0).
            if let Some(l) = loader.filter(|l| stack.top(Layer::Main) == Some(*l)) {
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({loader_id})")) {
                    stack.ready(l);
                    self.load_up = true;
                }
                if self.load_up {
                    let t = &mut fe.tips_loader;
                    let available = t.available();
                    let mut seed = t.seed;
                    let state = t.tips.update(seconds, available, &mut || crate::fe::tips_loader_host::lcg(&mut seed));
                    t.seed = seed;
                    if let Some(state) = state {
                        set_state(stage, loader_id, state.code() as i32, &mut fe)?;
                    }
                }
            }
            // SpawnSkater: loaded -> SetState(State_UpdateList) builds the list (NOT RETAIL YET: who posts retail's first
            // one is not read); intro complete = ready; then the picks and keys (b543 / b545).
            if let Some(s) = spawn.filter(|s| stack.top(Layer::Main) == Some(*s)) {
                if take_call(&mut fe, &format!("Game.OnScreenLoaded({spawn_id})")) {
                    fe.spawn_skater.states.push(fe.spawn_skater.rules.state_list);
                }
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({spawn_id})")) {
                    stack.ready(s);
                }
            }
            // Free Play options: loaded -> every row refreshes and the selection shows (NOT RETAIL YET: the poster of
            // the first states is not read); intro complete = ready; then the key states.
            if let Some(f) = free.filter(|f| stack.top(Layer::Main) == Some(*f)) {
                if take_call(&mut fe, &format!("Game.OnScreenLoaded({free_id})")) {
                    let first = [0, 1, 2, fe.free_skate.rules.state_selection];
                    fe.free_skate.states.extend(first);
                }
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({free_id})")) {
                    stack.ready(f);
                }
            }
            for state in std::mem::take(&mut fe.free_skate.states) {
                set_state(stage, free_id, state, &mut fe)?;
            }
            sounds.append(&mut fe.spawn_skater.sounds);
            for state in std::mem::take(&mut fe.spawn_skater.states) {
                set_state(stage, spawn_id, state, &mut fe)?;
            }
            for action in std::mem::take(&mut fe.spawn_skater.actions) {
                use skate_core::spawn_skater::Action;
                log.push(format!("spawn skater {action:?}"));
                match action {
                    Action::Close => {
                        if let Some(s) = spawn {
                            stack.request(Layer::Main, Request::Close { screen: s });
                        }
                    }
                    // A standard popup with its Continue option (the small popup shows no option, see CHEAT_POPUP).
                    Action::LimitReached => {
                        let [description, title, button] = fe.spawn_skater.rules.limit_popup.clone();
                        self.popups.push(PopupRequest { title, description, options: vec![button], callback: SPAWN_SKATER_POPUP.into(), ..Default::default() });
                        self.popup_dirty = true;
                    }
                    // FlowMessage 0x89 (82705E88, b562): fade, reload the player record, close FE 62. NOT RETAIL YET: no
                    // fade, the pause menu closes with it (inferred: Free Play resumes). skate_vita: only You is listed.
                    Action::SetPlayer(_) => {
                        spawn_out.push(action);
                        for s in [spawn, pause].into_iter().flatten() {
                            stack.request(Layer::Main, Request::Close { screen: s });
                        }
                    }
                    // Flow 0x8A: the Free Play options FE 34 over FE 62.
                    Action::Options => match free {
                        Some(f) => stack.request(Layer::Main, Request::push(f)),
                        None => log.push("no Free Play options screen (FE 34) in the tables".into()),
                    },
                    // Called / dismissed skaters go to the game's living world; the preview is not ported.
                    other => spawn_out.push(other),
                }
            }
            for action in std::mem::take(&mut fe.extras.selected) {
                use skate_core::extras::ExtrasAction;
                log.push(format!("extras select {action:?}"));
                match action {
                    // Row 0 (0x826558E8): text entry for a cheat code, titled with the row's label; push FE 46.
                    ExtrasAction::CheatEntry => {
                        if let Some(e) = entry {
                            let label = fe.extras.rules.rows.first().map_or("ID_EXTRA_CHEAT_CODE".to_owned(), |r| r.label.clone());
                            self.pending_entry = Some(skate_core::text_entry::TextEntryRequest::cheat_code(&label));
                            stack.request(Layer::Main, Request::push(e));
                        }
                    }
                    // Rows 1-3 (upstream: unlock directly): the grants go to the game's unlock store, the popup says it
                    // worked.
                    ExtrasAction::Grant { redeem, grants } => {
                        for unlock in std::iter::once(redeem).chain(grants) {
                            log.push(format!("unlock {} {}", unlock.kind, unlock.name));
                            match skate_core::unlocks::UnlockKind::from_extras(&unlock.kind) {
                                Some(kind) => unlocks_out.push((kind, unlock.name)),
                                // Challenges go to the progression manager in retail (not ported yet).
                                None => log.push(format!("unlock {} not for the unlock store", unlock.kind)),
                            }
                        }
                        self.popups.push(PopupRequest {
                            title: "ID_VALID_CODE_TITLE".into(),
                            description: "ID_VALID_CODE_DESC".into(),
                            options: vec!["ID_COMMON_OK".into()],
                            callback: CHEAT_POPUP.into(),
                            ..Default::default()
                        });
                        self.popup_dirty = true;
                    }
                    // Row 5: the text viewer FE 3 with our licenses (retail: the web browser's license file).
                    ExtrasAction::TextViewer { title, body } => {
                        if let Some(v) = viewer {
                            let text = match body.as_str() {
                                "licenses" => crate::fe::extras_host::LICENSES.to_owned(),
                                other => other.strip_prefix('#').unwrap_or(other).to_owned(),
                            };
                            let mut shown = skate_core::text_viewer::TextViewer::open(&title, &text, &Default::default());
                            // Ours: a line break is its own word. The movie ends a line only on a word that is
                            // "\n"; retail's split keeps the break at the start of the next word, which with the
                            // licenses' single line breaks would run every entry into the next.
                            shown.words = shown
                                .words
                                .into_iter()
                                .flat_map(|w| match w.strip_prefix('\n') {
                                    Some(rest) if !rest.is_empty() => vec!["\n".to_owned(), rest.to_owned()],
                                    _ => vec![w],
                                })
                                .collect();
                            fe.tos.viewer = Some(shown);
                            stack.request(Layer::Main, Request::push(v));
                        }
                    }
                    // Greyed until their screens are ported (doc 31).
                    other => log.push(format!("extras: {other:?} has no handler yet")),
                }
            }
            // The system keyboard's answer (FE 46 OK path 0x82639890 / cancel 0x82639B00).
            if let (Some(request), Some(answer)) = (self.entry.clone(), self.entry_answer.take()) {
                use skate_core::text_entry::EntryResult;
                let allowed = fe.language.get("ALL_VALID_ALPHANUMERIC_CHARACTERS").cloned().unwrap_or_default();
                let close_entry = |stack: &mut FeStack| {
                    if let Some(e) = entry {
                        stack.request(Layer::Main, Request::Close { screen: e });
                    }
                };
                match answer.map(|text| skate_core::text_entry::finish(&request, &text, &allowed)) {
                    None => close_entry(stack),
                    Some(EntryResult::Accepted(text)) => {
                        close_entry(stack);
                        if request.callback == skate_core::text_entry::CHEAT_CALLBACK {
                            // The matcher (b304) on the game's flags: entering a code toggles what is in effect.
                            let (outcome, dialog) = skate_core::cheats::enter(&text, &self.cheats, &fe.context.mode, &mut self.cheats_on);
                            log.push(format!("cheat code entered: {outcome:?}"));
                            if let skate_core::cheats::CheatOutcome::Toggled { id, on } = outcome {
                                cheats_out.push((id, on));
                            }
                            self.popups.push(PopupRequest {
                                title: dialog.title,
                                description: dialog.text,
                                options: vec![dialog.button],
                                    callback: CHEAT_POPUP.into(),
                                ..Default::default()
                            });
                            self.popup_dirty = true;
                        }
                    }
                    // Characters outside the set: retail's warning popup (record 0x107), then the keyboard again.
                    Some(EntryResult::Invalid) => {
                        use skate_core::text_entry::{INVALID_BUTTON, INVALID_TEXT, INVALID_TITLE};
                        self.popups.push(PopupRequest {
                            title: INVALID_TITLE.into(),
                            description: INVALID_TEXT.into(),
                            options: vec![INVALID_BUTTON.into()],
                            callback: CHEAT_POPUP.into(),
                            ..Default::default()
                        });
                        self.popup_dirty = true;
                        self.entry_ask = true;
                    }
                    Some(EntryResult::Reopen) => self.entry_ask = true,
                }
            }
            // Ask the game for text while FE 46 is on top and no popup is up.
            if self.entry_ask && self.popups.shown().is_none() && !self.popup_dirty && entry.is_some_and(|e| stack.top(Layer::Main) == Some(e)) {
                if let Some(request) = &self.entry {
                    self.entry_ask = false;
                    *text_entry = Some((crate::fe::fe_language::template(fe.language, &request.title).to_owned(), request.clone()));
                }
            }
            // Crossbar IntroComplete (vtable 0x82303F90 +104 = 8261B1F0) selects the mode menu (8261B9E8), then
            // posts SetState(InitialiseMenu) through the facade (+56 vt+20). The screen is ready (state 3) from here.
            if let Some(p) = pause.filter(|p| stack.top(Layer::Main) == Some(*p)) {
                if take_call(&mut fe, &format!("Game.OnScreenIntroComplete({crossbar_id})")) {
                    stack.ready(p);
                    fe.state = CrossbarState::open(&fe.view);
                    set_state(stage, crossbar_id, self.rules.state_initialise_menu, &mut fe)?;
                }
            }
            Ok(())
        })();
        // Audio.PlaySound(FE_SOUND_X) from the movies: the value is the fe record name (RegisterAudioEnum).
        for call in fe.calls.iter().filter_map(|c| c.strip_prefix("Audio.PlaySound(")) {
            let name = call.trim_end_matches(')').split(',').next().unwrap_or("");
            if !name.is_empty() && name != "undefined" {
                out.sounds.push(name.to_owned());
            }
        }
        self.crossbar = fe.state;
        self.settings = fe.settings;
        self.image_slots = std::mem::take(&mut fe.image_slots);
        self.popup = std::mem::take(&mut fe.popup);
        self.trick_guide = std::mem::take(&mut fe.trick_guide);
        self.replay_menu = std::mem::take(&mut fe.replay_menu);
        self.object_dropper = std::mem::take(&mut fe.object_dropper);
        self.browser = std::mem::take(&mut fe.browser);
        self.park_setup = std::mem::take(&mut fe.park_setup);
        self.cac = std::mem::take(&mut fe.cac);
        self.team = std::mem::take(&mut fe.team);
        self.extras = std::mem::take(&mut fe.extras);
        self.tos = std::mem::take(&mut fe.tos);
        self.offline_profile = std::mem::take(&mut fe.offline_profile);
        self.spawn_skater = std::mem::take(&mut fe.spawn_skater);
        self.free_skate = std::mem::take(&mut fe.free_skate);
        self.tips_loader = std::mem::take(&mut fe.tips_loader);
        fe.challenge_map.markers = if fe.challenge_map.open { fe.challenge_map.state.markers(fe.registry, fe.context) } else { Vec::new() };
        self.challenge_map = std::mem::take(&mut fe.challenge_map);
        let settings_selected = std::mem::take(&mut self.settings.selected);
        // A Challenge Map leaf the player picked goes through the registry: the engine's teleport handler names the
        // location for the game and closes the map and the pause menu (retail fades out and closes the front end,
        // sub_826494C8); a greyed one never reaches here.
        if let Some(entry) = self.challenge_map.selected.take() {
            let outcome = self.menus.act(MenuAction::Select { entry: entry.clone() });
            out.log.push(format!("challenge map select {}: {outcome:?}", entry.hook_id()));
            if let Outcome::Forward(Owner::Engine(name), _) = &outcome {
                if let Some(key) = name.strip_prefix(TELEPORT) {
                    out.teleports.push(key.to_owned());
                    self.close();
                }
            }
        }
        let _registry = crate::fe::profile::span(crate::fe::profile::Span::Registry);
        if result.is_ok() {
            let mut refresh: Option<i32> = None;
            if let Some((item, sub)) = sub_selected {
                out.log.push(format!("select {item} sub-option {sub}"));
                out.sub_options.push((item, sub));
            }
            if let Some(item) = selected {
                // Retail runs the item (its FE screen); ours goes through the registry: an engine handler pushes a
                // ported FE screen; any other outcome goes to the game.
                let outcome = self.menus.act(MenuAction::Select { entry: EntryRef::item(&item) });
                out.log.push(format!("select {item}: {outcome:?}"));
                match &outcome {
                    Outcome::Forward(Owner::Engine(name), _) if name.starts_with(SCREEN) => match self.index(&name[SCREEN.len()..]) {
                        // The Trick Guide opens when the screen is black (crossbar case 0x8261BF78 holds the fade,
                        // callback 0x8261CDD8 pushes FE 14). NOT RETAIL YET: Game Settings opens without the fade.
                        Some(screen) if Some(screen) == self.index(TRICK_GUIDE_SCREEN) => {
                            let params = self.fade_params;
                            if let Some(action) = self.fade.hold(FADE_SCREEN, Some(FadeAction::Push(screen)), false, &params) {
                                run_fade_action(&mut self.stack, action);
                            }
                        }
                        Some(screen) => {
                            // The row sets FE 62 +0x40 before the push (b307: 200 Free Play, 100 Call Skater).
                            if Some(screen) == self.index(SPAWN_SKATER_SCREEN) {
                                if let Some((_, mode)) = self.spawn_skater.rules.items.iter().find(|(i, _)| *i == item) {
                                    self.spawn_skater_mode = *mode;
                                }
                            }
                            self.stack.request(Layer::Main, Request::push(screen))
                        }
                        None => out.log.push(format!("no FE screen {}", &name[SCREEN.len()..])),
                    },
                    _ => out.items.push((item, outcome)),
                }
            }
            // Settings OnMenuSelect (sub_8260D290): a link row opens its screen (+68) and rebuilds (SetState(1));
            // other rows go to their handler.
            for row in settings_selected {
                let Some(id) = self.settings.row_id(row).map(str::to_owned) else { continue };
                out.sounds.push("core_a_button".into());
                match self.menus.act(MenuAction::Select { entry: EntryRef::setting(&id) }) {
                    Outcome::OpenScreen(screen) => {
                        self.settings.screen = screen;
                        self.settings.row = 0;
                        refresh = Some(1);
                    }
                    // A row whose engine handler is a ported FE screen (row 25 Video Calibration: sub_82D0AEF0 pushes
                    // FE 51 with param 0, keep-open 1, keep-visible 0, so Game Settings stays open, hidden).
                    Outcome::Forward(Owner::Engine(name), _) if name.starts_with(SCREEN) => match self.index(&name[SCREEN.len()..]) {
                        Some(screen) => {
                            self.stack.request(Layer::Main, Request::Push { screen, param: 0, keep_open: true, keep_visible: false })
                        }
                        None => out.log.push(format!("no FE screen {}", &name[SCREEN.len()..])),
                    },
                    outcome => out.settings.push((id, outcome)),
                }
            }
            for (id, direction) in steps {
                let outcome = self.menus.act(MenuAction::Step { entry: EntryRef::setting(&id), direction });
                out.log.push(format!("settings step {id} {direction:?}: {outcome:?}"));
                out.settings.push((id, outcome));
                refresh = refresh.or(Some(50));
            }
            if let Some(state) = refresh {
                let mut fe = front_end(&self.menus, &self.language, &self.sub_options, self.crossbar.clone());
                fe.settings = std::mem::take(&mut self.settings);
                fe.settings_titles = self.settings_titles.clone();
                if let Err(e) = set_state(&mut self.stage, self.options_id, state, &mut fe) {
                    out.log.push(format!("settings refresh: {e}"));
                }
                self.settings = fe.settings;
            }
        }
        if self.stack.top(Layer::Main).is_none() {
            self.hud = FeHud::default();
        }
        if let Err(e) = result {
            // A stopped stage gives the game back: no suspended gameplay, the HUD on.
            out.log.push(format!("stage stopped: {e}"));
            self.failed = true;
            self.suspended = false;
            self.hud = FeHud::default();
        }
        out
    }

    /// The front end's draws in stage space (1280 x 720), back to front: image slots resolved, prompts and images the
    /// set lacks dropped, the FE fader over everything. Texture keys go to `movies().image(key)`; an empty key is a
    /// solid fill. Prompts (`prompt/<retail button>`) are dropped until the Vita's glyphs exist (NOT RETAIL YET).
    pub fn draw(&mut self) -> Vec<Draw> {
        if self.failed {
            return Vec::new();
        }
        let _span = crate::fe::profile::span(crate::fe::profile::Span::Draw);
        let mut levels = Vec::new();
        let mut draws = match self.stage.draw_levels(|key, movie| self.movies.scene(key, movie), &mut levels) {
            Ok(draws) => draws,
            Err(_) => {
                self.failed = true;
                return Vec::new();
            }
        };
        // Button prompts (CustomButton components, `prompt/<retail button>`) go to the host (its controller glyphs),
        // drawn over every movie: while the replay help covers the replay screen, only the help's own.
        self.prompts.clear();
        // A shown popup is on top of everything (only its movie draws among the popup movies); else the replay help.
        let help = movie_key(REPLAY_HELP_PATH);
        // The popup movies stay loaded and visible; the shown one is the last that drew anything.
        let end = |i: usize| levels.get(i + 1).map_or(draws.len(), |(_, next)| *next);
        let popup = (0..levels.len()).filter(|&i| levels[i].0.starts_with("source/screens/popup/") && end(i) > levels[i].1).map(|i| levels[i].1).max();
        let first = match popup.filter(|_| self.popups.shown().is_some()) {
            Some(i) => i,
            None if self.replay_menu.help_shown => levels.iter().find(|(k, _)| *k == help).map_or(0, |(_, i)| *i),
            None => 0,
        };
        drop(levels);
        for d in draws[first..].iter().filter(|d| d.texture.starts_with("prompt/") && d.multiply[3] > 0.0) {
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for v in &d.vertices {
                for i in 0..2 {
                    lo[i] = lo[i].min(v.position[i]);
                    hi[i] = hi[i].max(v.position[i]);
                }
            }
            // A prompt off the stage (the action overlay's idle button sits below it) gets no glyph.
            let on_stage = hi[0] >= 0.0 && lo[0] <= 1280.0 && hi[1] >= 0.0 && lo[1] <= 720.0;
            if lo[0] <= hi[0] && on_stage {
                let name = d.texture["prompt/".len()..].to_owned();
                self.prompts.push((name, [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, hi[0] - lo[0], hi[1] - lo[1]]));
            }
        }
        draws.retain(|d| !d.texture.starts_with("prompt/"));
        // Map markers (MapRenderComponent): a 32 x 32 screen-pixel quad per marker (`8268D960`, size 32 x scale 1.0)
        // centred on the map clip's translation plus its map point, highlighted last: retail's drawer (`825E2618`)
        // takes only the translation of the clip's matrix (`0x830B8600`), not its scale (b282). Nothing while the
        // Challenge Map is closed (the gate byte, b263).
        let mut expanded = Vec::with_capacity(draws.len());
        for draw in draws.drain(..) {
            if draw.texture == crate::fe::apt_scene::REPLAY_TIMELINE {
                expanded.extend(crate::fe::replay_menu_host::timeline_draws(&draw, &self.replay_menu.feed));
                continue;
            }
            if draw.texture == crate::fe::apt_scene::FENG_SHUI_TERRAIN || draw.texture == crate::fe::apt_scene::FENG_SHUI_PROPS {
                expanded.extend(self.object_dropper.bar_draws(&draw, draw.texture == crate::fe::apt_scene::FENG_SHUI_PROPS));
                continue;
            }
            if draw.texture != crate::fe::apt_scene::MAP_MARKERS {
                expanded.push(draw);
                continue;
            }
            let o = draw.vertices[0].position;
            self.challenge_map.origin = o;
            let at = |x: f32, y: f32, u: f32, v: f32| Vertex { position: [o[0] + x, o[1] + y], uv: [u, v] };
            for (icon, [x, y], _) in &self.challenge_map.markers {
                let (a, b, c, d) = (at(x - 16., y - 16., 0., 0.), at(x + 16., y - 16., 1., 0.), at(x + 16., y + 16., 1., 1.), at(x - 16., y + 16., 0., 1.));
                expanded.push(Draw {
                    texture: format!("{}texture/{icon}", crate::fe::retail_menu_movies::NAMED_IMAGE_PREFIX),
                    fill: draw.fill,
                    vertices: vec![a.clone(), b, c.clone(), a, c, d],
                    multiply: draw.multiply,
                    add: draw.add,
                });
            }
        }
        let mut draws = expanded;
        // Swatches (SwatchRenderComponent, `swatch/<n>`): a solid quad of the slot's colour; an unset slot draws nothing.
        {
            use crate::fe::object_dropper_host::SWATCH_TEXTURE_PREFIX;
            let swatches = &self.object_dropper.swatches;
            draws.retain_mut(|d| {
                let Some(slot) = d.texture.strip_prefix(SWATCH_TEXTURE_PREFIX) else { return true };
                let Some(colour) = slot.parse::<usize>().ok().and_then(|s| swatches.get(s).copied().flatten()) else { return false };
                d.texture.clear();
                d.fill = Fill::Solid;
                d.multiply = [colour[0], colour[1], colour[2], d.multiply[3]];
                d.add = [0.0; 4];
                true
            });
        }
        // Image slots (ImageRenderComponent, `slot/<n>`): the slot's loaded image; an empty slot or an image nobody
        // has draws nothing (retail: the texture lookup finds nothing, b258 section 2), never the white fallback.
        use crate::fe::image_renderer_host::{ASSET_TEXTURE_PREFIX, SLOT_TEXTURE_PREFIX};
        for draw in &mut draws {
            if draw.texture.starts_with(SLOT_TEXTURE_PREFIX) {
                draw.texture = self.image_slots.texture(&draw.texture).unwrap_or(SLOT_TEXTURE_PREFIX).to_owned();
            }
        }
        draws.retain(|d| {
            let named = [SLOT_TEXTURE_PREFIX, ASSET_TEXTURE_PREFIX, crate::fe::retail_menu_movies::NAMED_IMAGE_PREFIX]
                .iter()
                .any(|p| d.texture.starts_with(p));
            // Slappy's thumbnails are not in the menu set: the game supplies them (skate_vita: `cas.pak`).
            !named || d.texture.starts_with(CAS_THUMB_PREFIX) || self.movies.image(&d.texture).is_some()
        });
        // The FE fader over everything (alpha 1 = black), past the stage edges for any aspect ratio.
        if self.fade.alpha > 0.0 {
            let v = |x: f32, y: f32| Vertex { position: [x, y], uv: [0.0, 0.0] };
            let (x0, y0, x1, y1) = (-1280.0, -720.0, 2560.0, 1440.0);
            draws.push(Draw {
                texture: String::new(),
                vertices: vec![v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y0), v(x1, y1), v(x0, y1)],
                multiply: [0.0, 0.0, 0.0, self.fade.alpha],
                add: [0.0; 4],
                fill: Fill::Solid,
            });
        }
        draws
    }

    /// The HUD phone's skate.Park rows: the font and the strings of Edit Park and Reset All Props.
    pub fn phone_text(&self) -> Option<(&crate::fe::apt_text::Font, &str, &str)> {
        let text = |key: &str| self.language.get(key).map(String::as_str);
        Some((self.shadow_font.as_ref()?, text("ID_PHONELIST_EDIT_PARK")?, text("ID_PHONELIST_RESET_ALL_OBJECTS")?))
    }

    /// A Challenge Map location's teleport: its map and locator matrix (rows right, up, forward, translation).
    pub fn teleport(&self, key: &str) -> Option<&(String, [[f32; 4]; 4])> {
        self.teleports.get(key)
    }

    /// Gives the Challenge Map locations whose map `can_travel` accepts the engine's teleport handler (enabled);
    /// the others stay greyed (skate-game `RetailMenus::register_teleports`). Returns how many.
    pub fn register_teleports(&mut self, can_travel: &dyn Fn(&str) -> bool) -> usize {
        let locations: Vec<EntryRef> = self
            .menus
            .registry
            .data()
            .map_nodes
            .iter()
            // Locations, and the skate.Park folder's park worlds (keyed by the world row's id: the bake gives each the
            // first destination that names it).
            .filter(|n| matches!(n.entry.kind, skate_core::menus::EntryKind::FeLocation | skate_core::menus::EntryKind::World))
            .filter(|n| self.teleports.get(&n.entry.id).is_some_and(|(map, _)| can_travel(map)))
            .map(|n| n.entry.clone())
            .collect();
        for entry in &locations {
            let owner = Owner::Engine(format!("{TELEPORT}{}", entry.id));
            self.menus.registry.register_handler(entry.clone(), Handler { owner, select: true, highlight: false, callback: None });
        }
        locations.len()
    }

    /// The Trick Guide's 3D demo while its screen is on top (upstream's `TrickGuideDemo`): the highlighted entry's
    /// clip (S+204; empty = keep the current one) and camera track (S+200).
    pub fn trick_guide_demo(&self) -> Option<(&str, i16)> {
        let top = self.stack.top(Layer::Main)?;
        (Some(top) == self.index(TRICK_GUIDE_SCREEN)).then(|| (self.trick_guide.state.animation.as_str(), self.trick_guide.state.camera_track))
    }

    /// The teleporter prompt (the action overlay) with this title, or hidden; posted on the next update.
    pub fn set_signup(&mut self, title: Option<String>) {
        self.signup_title = title;
    }

    /// skate_vita: the save wheel on (a memory card write runs) or off; applied on the next update.
    pub fn set_save_tag(&mut self, on: bool) {
        self.save_tag = on;
    }

    /// The save wheel is wanted or still up: the stage must update and draw.
    pub fn save_tag_active(&self) -> bool {
        self.save_tag || self.save_tag_shown
    }

    /// The action overlay is in this install (else the game shows its own prompt).
    pub fn has_signup(&self) -> bool {
        self.signup_id.is_some()
    }

    /// The prompt shows or was just hidden (its outro still drawing): the stage must update and draw.
    pub fn signup_active(&self) -> bool {
        self.signup_shown.is_some() || self.signup_title.is_some()
    }

    /// The cheats in effect (the game's flags), so a code toggles what is on.
    pub fn set_cheats(&mut self, on: &[&str]) {
        self.cheats_on.on = on.iter().map(|s| (*s).to_owned()).collect();
    }

    /// The pros Call Skater lists ([`crate::fe::spawn_skater_host::callable`]) and how many may be called at once
    /// (retail 3; the Vita's own cap).
    pub fn set_callable(&mut self, callable: Vec<crate::fe::spawn_skater_host::Callable>, limit: usize) {
        self.callable = callable;
        let rules = &mut self.spawn_skater.rules;
        rules.companion_limit = limit;
        if let Some(text) = crate::fe::spawn_skater_host::limit_text(limit) {
            rules.limit_popup[0] = text;
        }
    }

    /// The skaters called now (the living world's companions, oldest first); FE 62 reads them when it opens.
    pub fn set_called(&mut self, called: &[String]) {
        if self.called != called {
            self.called = called.to_vec();
        }
    }

    /// The retail loading screen can show (its movie, template and lines are in this install).
    pub fn has_loading_screen(&self) -> bool {
        self.tips_loader_id.is_some()
    }

    /// The game loads a world (true from the start of the load until the world is ready): the loading screen's
    /// sequence runs on the next updates.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Any part of the loading screen's sequence runs (the stage must be updated and drawn).
    pub fn load_active(&self) -> bool {
        self.load.active()
    }

    /// The loading screen covers the world (the fader black or FE 52 up): draw black under it, not the world.
    pub fn load_covered(&self) -> bool {
        use skate_core::tips_loader::LoadPhase;
        matches!(self.load.phase, LoadPhase::Opening | LoadPhase::Up | LoadPhase::Closing | LoadPhase::Closed)
    }

    /// The career stats the loading screen's Stats panel shows from its next open, and Game Settings > Units (feet when
    /// imperial).
    pub fn set_career_stats(&mut self, stats: &skate_core::career_stats::CareerStats, imperial: bool) {
        self.career_stats.clone_from(stats);
        self.imperial = imperial;
    }

    /// The session's Free Play values FE 34 opens with (Traffic, Pedestrians, A.I. Skaters).
    pub fn set_free_play(&mut self, values: skate_core::living_world::FreePlay) {
        self.free_play = values;
    }

    /// skate_vita: opens the shared browser on parks (skate.Park > My Parks, FE 0x3B) with the saved parks; false
    /// without its movie.
    pub fn open_browser(&mut self, items: Vec<crate::fe::browser_host::BrowserItem>) -> bool {
        if self.browser_id.is_none() || self.index(BROWSER_SCREEN).is_none() {
            return false;
        }
        self.browser_open = Some(items);
        true
    }

    /// skate_vita: the saved parks changed (a delete): the browser shows them.
    pub fn set_browser_items(&mut self, items: Vec<crate::fe::browser_host::BrowserItem>) {
        self.browser_items = Some(items);
    }

    /// skate_vita: what the player did in the browser since the last call (select or delete a saved park).
    pub fn take_browser_events(&mut self) -> Vec<crate::fe::browser_host::BrowserEvent> {
        std::mem::take(&mut self.browser.events)
    }

    /// skate_vita: the browser is on the stack.
    pub fn browser_open(&self) -> bool {
        self.browser.open || self.browser_open.is_some()
    }

    /// skate_vita: opens Park Setup (skate.Park > New Park, FE 0x3A) with the lots; false without its movie.
    pub fn open_park_setup(&mut self, lots: Vec<crate::fe::park_setup_host::ParkSetupLot>) -> bool {
        if self.park_setup_id.is_none() || self.index(PARK_SETUP_SCREEN).is_none() {
            return false;
        }
        self.park_setup_open = Some(lots);
        true
    }

    /// skate_vita: opens the character customisation (FE 15) on a skater; false without its movie.
    pub fn open_cac(&mut self, screen: skate_core::cac::Cac, cash: u32) -> bool {
        if self.cac_id.is_none() || self.index(CAC_SCREEN).is_none() {
            return false;
        }
        self.cac_open = Some((screen, cash));
        true
    }

    /// skate_vita: the customisation's skater when its mesh is out of date (once per change), to build and show.
    pub fn cac_look(&mut self) -> Option<(skate_core::skater_record::SkaterRecord, skate_core::cac::LookChange)> {
        let c = self.cac.screen.as_mut()?;
        let change = c.look.take()?;
        Some((c.shown_record().into_owned(), change))
    }

    /// skate_vita: the tattoos' shader rows of the customisation's skater (the placement moves without a rebuild).
    pub fn cac_decal_rows(&self) -> Option<[[[f32; 3]; 2]; 5]> {
        self.cac.screen.as_ref().map(|c| c.decal_rows())
    }

    /// skate_vita: the customisation's mesh is built: its keys go on.
    pub fn cac_look_done(&mut self) {
        if let Some(c) = self.cac.screen.as_mut() {
            c.look_done();
        }
    }

    /// skate_vita: what the customisation's preview draws, while it is open.
    pub fn cac_preview(&self) -> Option<skate_core::cac::Preview> {
        self.cac.screen.as_ref().filter(|_| self.cac.shown)?.preview()
    }

    /// skate_vita: opens the team screen (FE 67) on the career team; false without its movie.
    pub fn open_team(&mut self, team: skate_core::team::Team) -> bool {
        if self.team_id.is_none() || self.index(TEAM_SCREEN).is_none() {
            return false;
        }
        self.team_open = Some(team);
        true
    }

    /// skate_vita: a name typed for the team screen (`member` None: the team's).
    pub fn team_named(&mut self, member: Option<usize>, text: &str) {
        if let Some(t) = self.team.screen.as_mut() {
            t.named(member, text);
        }
    }

    /// skate_vita: the team screen's names (the team's, the members'), while it is open.
    pub fn team(&self) -> Option<&skate_core::team::Team> {
        self.team.screen.as_ref()
    }

    /// skate_vita: the skater the player left the customisation with, once.
    pub fn take_cac(&mut self) -> Option<skate_core::skater_record::SkaterRecord> {
        self.cac.finished.take()
    }

    /// skate_vita: the lot and template the player took in Park Setup since the last call.
    pub fn take_park_setup_choices(&mut self) -> Vec<crate::fe::park_setup_host::ParkSetupChoice> {
        std::mem::take(&mut self.park_setup.choices)
    }

    /// skate_vita: a string id in the menus' language (the game's own keyboard titles).
    pub fn text(&self, id: &str) -> String {
        crate::fe::fe_language::template(&self.language, id).to_owned()
    }

    /// The game's answer to [`Frame::text_entry`]: the text typed, or None (cancelled). Taken on the next update.
    pub fn text_entered(&mut self, text: Option<String>) {
        self.entry_answer = Some(text);
    }

    /// Raises a popup of the game's (the replay's Finished menu): shown on the next update over whatever is open, it
    /// takes the keys until chosen; the choice comes back in [`Frame::popup_choices`] under the request's callback.
    pub fn push_popup(&mut self, request: PopupRequest) -> i32 {
        let id = self.popups.push(request);
        self.popup_dirty = true;
        id
    }

    /// Takes the game's popups with this callback down (the screen they belonged to closed).
    pub fn remove_popups(&mut self, callback: &str) {
        if self.popups.remove_callback(callback) {
            self.popup_dirty = true;
        }
    }

    /// A popup is shown (it takes the keys).
    pub fn popup_shown(&self) -> bool {
        self.popups.shown().is_some()
    }

    /// The game's instant replay this frame (None: no replay open): the next update opens the retail replay screen,
    /// feeds it, opens its help with the feed and closes it with the replay.
    pub fn set_replay(&mut self, feed: Option<crate::fe::replay_menu_host::ReplayScreenFeed>) {
        self.replay_feed = feed;
    }

    /// The Object Dropper this frame (None: closed) and the screen modes its editor posted: the next update opens the
    /// retail screen, feeds it and posts the modes, and closes it with the editor.
    pub fn set_object_dropper(&mut self, feed: Option<crate::fe::object_dropper_host::ObjectDropperFeed>, posts: &[u8], quick_posts: &[u8]) {
        self.object_dropper_feed = feed;
        self.object_dropper_posts.extend_from_slice(posts);
        self.object_quick_posts.extend_from_slice(quick_posts);
    }

    /// The Object Dropper movie is still on the stage (after a close, until its outro ends and the screen manager
    /// unloads it).
    pub fn object_dropper_loaded(&self) -> bool {
        let key = movie_key(OBJECT_DROPPER_PATH);
        self.stage.levels().iter().any(|l| l.key == key)
    }

    /// The Quick Menu's Style / Branding list callbacks since the last call: (row, selected).
    pub fn take_object_dropper_list_calls(&mut self) -> Vec<(usize, bool)> {
        std::mem::take(&mut self.object_dropper.list_calls)
    }

    /// The retail Object Dropper screen can show (its template and movie are in this install).
    pub fn has_object_dropper(&self) -> bool {
        self.object_dropper_id.is_some()
    }

    /// The retail replay screen can show (its template and movie are in this install).
    pub fn has_replay(&self) -> bool {
        self.replay_id.is_some()
    }

    /// The retail replay screen is shown (its intro completed); the help over it.
    pub fn replay_shown(&self) -> bool {
        self.replay_shown
    }

    pub fn replay_help_shown(&self) -> bool {
        self.replay_menu.help_shown
    }

    /// The crossbar's highlighted row (tests).
    pub fn crossbar_option(&self) -> i32 {
        self.crossbar.option
    }

    /// The button prompts of the last [`Self::draw`] (retail button name, centre x / y, width, height in stage space).
    pub fn prompts(&self) -> &[(String, [f32; 4])] {
        &self.prompts
    }

    /// The image behind a draw's texture key.
    pub fn image(&self, texture: &str) -> Option<MenuImage<'_>> {
        self.movies.image(texture)
    }
}

/// A Video Calibration step: post the page, close the screen, or push the "finished" popup (0x8260E950: Sk8Popup,
/// priority 0, not cancelable, Yes / No; its choice comes back through `VIDEO_CALIBRATION_POPUP`).
#[allow(clippy::too_many_arguments)]
fn run_calibration(
    stage: &mut Stage,
    stack: &mut FeStack,
    popups: &mut PopupList,
    popup_dirty: &mut bool,
    rules: &CalibrationRules,
    id: i32,
    action: CalibrationAction,
    screen: usize,
    fe: &mut FrontEnd,
    log: &mut Vec<String>,
) -> Result<(), String> {
    match action {
        CalibrationAction::SetState(page) => set_state(stage, id, page as i32, fe),
        CalibrationAction::Close => {
            stack.request(Layer::Main, Request::Close { screen });
            Ok(())
        }
        CalibrationAction::Popup => {
            let id = popups.push(PopupRequest {
                title: rules.popup_title.clone(),
                description: rules.popup_description.clone(),
                options: rules.popup_buttons.clone(),
                callback: VIDEO_CALIBRATION_POPUP.into(),
                ..Default::default()
            });
            log.push(format!("Video Calibration finished: popup {id}"));
            *popup_dirty = true;
            Ok(())
        }
    }
}

/// FE vt+24 (`sub_825EC530`) and host 29 show (`0x825EF440`): hide every popup movie, then for the list's last entry
/// SetScreenState(1) and ShowScreen on its kind's movie, play core_popup and take input. An empty list hides.
fn show_popup(
    stage: &mut Stage,
    popups: &PopupList,
    ids: &[(PopupKind, i32)],
    fe: &mut FrontEnd,
    sounds: &mut Vec<String>,
    log: &mut Vec<String>,
) -> Result<(), String> {
    for (_, id) in ids {
        show_screen(stage, *id, false, fe)?;
    }
    fe.popup.shown = None;
    fe.popup.input_ready = false;
    let Some((popup, request)) = popups.shown() else { return Ok(()) };
    let Some(handle) = ids.iter().find(|(k, _)| *k == request.kind).map(|(_, id)| *id) else {
        log.push(format!("popup {popup} has no movie ({})", request.kind.template()));
        return Ok(());
    };
    // small_popup reads no options and answers only B when cancelable (doc 31 "Popup kinds").
    if request.kind == PopupKind::Small && !request.cancelable && !request.options.is_empty() {
        log.push(format!("popup {popup} ({}) is small with options but not cancelable: no press can close it", request.callback));
    }
    fe.popup.shown = Some(request.clone());
    set_state(stage, handle, 1, fe)?;
    show_screen(stage, handle, true, fe)?;
    sounds.push("core_popup".into());
    fe.popup.input_ready = true;
    log.push(format!("popup {popup} shown ({}, screen {handle}, {})", request.kind.template(), request.callback));
    Ok(())
}

/// The Skate Profile's queued screen states, once its movie loaded.
fn flush_offline_profile(stage: &mut Stage, id: i32, fe: &mut FrontEnd) -> Result<(), String> {
    if !fe.offline_profile.loaded {
        return Ok(());
    }
    for state in std::mem::take(&mut fe.offline_profile.states) {
        set_state(stage, id, state, fe)?;
    }
    Ok(())
}

/// The autosave layer's corner wheel while the loading screen is up, as in the recomp's loads (doc 31 "Loading screen
/// (FE 52)"). A menu set without the autosave layer has no such function; the load runs without the wheel.
fn load_tag(stage: &mut Stage, on: bool, fe: &mut FrontEnd, log: &mut Vec<String>) {
    let (path, args) = if on { ("_global.Load_EnableTag", vec![Value::Bool(true)]) } else { ("_global.Load_HideTag", Vec::new()) };
    if let Err(e) = stage.call_path(path, args, fe) {
        log.push(format!("{path}: {e}"));
    }
}

/// The autosave layer's save wheel in the corner (`Save_EnableTag(style, isdelete)`, style "false": the corner wheel, not
/// the central panel) while the game writes to the memory card (owner, on retail: saving or deleting a skate.Park shows
/// it until done). NOT RETAIL YET: the style retail passes for parks is not read.
fn save_tag(stage: &mut Stage, on: bool, fe: &mut FrontEnd, log: &mut Vec<String>) {
    let (path, args) = if on { ("_global.Save_EnableTag", vec![Value::Text("false".into()), Value::Bool(false)]) } else { ("_global.Save_HideTag", Vec::new()) };
    log.push(format!("save tag {}", if on { "on" } else { "off" }));
    if let Err(e) = stage.call_path(path, args, fe) {
        log.push(format!("{path}: {e}"));
    }
}

fn open_screen(stage: &mut Stage, path: &str, template: &str, fe: &mut FrontEnd) -> Result<(), String> {
    open_screen_on(stage, path, "screen", template, fe)
}

/// OpenScreen bridge (0x82D09140): path, AS layer ("screen" unless the caller names one), the template's visible flag
/// ("0" for every menu template here), template name.
fn open_screen_on(stage: &mut Stage, path: &str, layer: &str, template: &str, fe: &mut FrontEnd) -> Result<(), String> {
    let args = [format!("source/screens/{path}"), layer.into(), "0".into(), template.into()];
    stage.call_path("_global.ScreenManager.OpenScreen", args.map(Value::Text).to_vec(), fe)?;
    Ok(())
}

fn run_fade_action(stack: &mut FeStack, action: FadeAction) {
    match action {
        FadeAction::Push(screen) => stack.request(Layer::Main, Request::push(screen)),
        FadeAction::Close(screen) => stack.request(Layer::Main, Request::Close { screen }),
    }
}

/// The Challenge Map's queued screen states (`SetState(n)`), and its close at the root (B).
fn flush_challenge_map(stage: &mut Stage, stack: &mut FeStack, map: Option<usize>, id: i32, fe: &mut FrontEnd) -> Result<(), String> {
    for state in std::mem::take(&mut fe.challenge_map.states) {
        set_state(stage, id, state, fe)?;
    }
    if std::mem::take(&mut fe.challenge_map.close) {
        if let Some(screen) = map {
            stack.request(Layer::Main, Request::Close { screen });
        }
    }
    Ok(())
}

/// The Trick Guide's queued screen states (its picker callbacks post them from inside a movie call), in order; the
/// rebuild they start can queue more (bounded).
fn flush_trick_guide(stage: &mut Stage, id: i32, fe: &mut FrontEnd) -> Result<(), String> {
    for _ in 0..8 {
        let states = std::mem::take(&mut fe.trick_guide.states);
        if states.is_empty() {
            return Ok(());
        }
        for state in states {
            set_state(stage, id, state, fe)?;
        }
    }
    Ok(())
}

/// The C++ SetScreenState through the facade: both as "%d" strings (sub_8263F320 index 10).
fn set_state(stage: &mut Stage, id: i32, state: i32, fe: &mut FrontEnd) -> Result<(), String> {
    stage.call_path("_global.ScreenManager.SetScreenState", vec![Value::Text(id.to_string()), Value::Text(state.to_string())], fe)?;
    Ok(())
}

fn show_screen(stage: &mut Stage, id: i32, visible: bool, fe: &mut FrontEnd) -> Result<(), String> {
    let visible = if visible { "true" } else { "false" };
    stage.call_path("_global.ScreenManager.ShowScreen", vec![Value::Text(id.to_string()), Value::Text(visible.into())], fe)?;
    Ok(())
}

fn update_input(stage: &mut Stage, id: i32, key: u32, controller: usize, fe: &mut FrontEnd) -> Result<(), String> {
    let args = [id.to_string(), key.to_string(), controller.to_string()];
    stage.call_path("_global.ScreenManager.UpdateInput", args.map(Value::Text).to_vec(), fe)?;
    Ok(())
}

/// Whether the movie made this call this frame (removes it).
fn take_call(fe: &mut FrontEnd, call: &str) -> bool {
    let before = fe.calls.len();
    fe.calls.retain(|c| c != call);
    fe.calls.len() != before
}

fn close(stage: &mut Stage, id: i32, fe: &mut FrontEnd) -> Result<(), String> {
    // Retail CloseScreen (sub_82D09298): the AS id as "%d"; the screen plays its outro.
    stage.call_path("_global.ScreenManager.CloseScreen", vec![Value::Text(id.to_string())], fe)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_reopened_and_closed_in_one_batch_is_neither() {
        let fx = vec![Effect::Close(7), Effect::SuspendGameplay(true), Effect::Open(3), Effect::Close(3), Effect::SuspendGameplay(false)];
        assert_eq!(drop_reopen_and_close(fx), [Effect::Close(7), Effect::SuspendGameplay(true), Effect::SuspendGameplay(false)]);
        assert_eq!(drop_reopen_and_close(vec![Effect::Close(3), Effect::Open(3)]), [Effect::Close(3), Effect::Open(3)]);
    }

    /// The pause front end on the lean set: Start opens the pause menu (gameplay suspended, the crossbar draws), the
    /// crossbar walks to Game Settings and A opens it with its rows, Left steps a row, B twice and Start close
    /// everything (gameplay resumed). Data-gated: `SKATE3_MENU_MOVIES=<set>` and `SKATE3_MAIN_CHECKOUT=<install>`.
    #[test]
    fn pause_menu_opens_game_settings_and_closes() {
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        eprintln!("{} movies: {:?}", movies.movies.len(), movies.movies.keys().collect::<Vec<_>>());
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        assert!(ms.has_pause());
        eprintln!("levels after boot: {:?}", ms.stage.levels().iter().map(|l| l.key.as_str()).collect::<Vec<_>>());
        eprintln!("popups {:?}, unhandled {:?}", ms.popup_ids, ms.stage.unhandled());
        let run = |ms: &mut MenuStage, keys: Vec<(u32, usize)>, frames: usize| {
            let mut log = Vec::new();
            let mut first = ms.update_keys(keys, 1.0 / 60.0);
            log.append(&mut first.log);
            for _ in 1..frames {
                log.append(&mut ms.update_keys(Vec::new(), 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        run(&mut ms, Vec::new(), 5);
        assert!(!ms.open() && !ms.suspended());
        let log = run(&mut ms, vec![(APT_START, 0)], 90);
        assert!(ms.open() && ms.suspended(), "{log:?}");
        assert!(log.iter().any(|l| l.starts_with("open pause menu")), "{log:?}");
        let draws = ms.draw();
        assert!(draws.len() > 20, "the crossbar draws: {}", draws.len());
        // The crossbar's tab and row with Game Settings: Right to the tab, Down to the row, A.
        let view = front_end(&ms.menus, &ms.language, &ms.sub_options, ms.crossbar.clone()).view;
        let (tab, row) = (0..8)
            .flat_map(|t| (0..16).map(move |r| (t, r)))
            .find(|&(t, r)| view.item(t, r) == Some("GameSettings"))
            .expect("GameSettings in the pause menu");
        for _ in 0..8 {
            if ms.crossbar.menu == tab as i32 {
                break;
            }
            run(&mut ms, vec![(APT_RIGHT, 0)], 20);
        }
        for _ in 0..16 {
            if ms.crossbar.option == row as i32 {
                break;
            }
            run(&mut ms, vec![(13, 0)], 10);
        }
        assert_eq!((ms.crossbar.menu, ms.crossbar.option), (tab as i32, row as i32));
        let log = run(&mut ms, vec![(APT_NEXT, 0)], 90);
        assert!(log.iter().any(|l| l.starts_with("open Game Settings")), "{log:?}");
        assert!(!ms.settings.rows.is_empty(), "the rows were built");
        let frame = ms.update_keys(vec![(APT_LEFT, 0)], 1.0 / 60.0);
        assert!(frame.settings.iter().any(|(id, _)| ms.settings.rows.contains(id)), "{:?}", frame.settings);
        run(&mut ms, Vec::new(), 30);
        run(&mut ms, vec![(APT_PREVIOUS, 0)], 60);
        let log = run(&mut ms, vec![(APT_START, 0)], 90);
        assert!(!ms.open() && !ms.suspended(), "{log:?}");
        // The host closes everything and opens again at once (skate_vita: the pause menu's Main menu row): one update
        // carries the close out, then Start opens the menu again.
        run(&mut ms, vec![(APT_START, 0)], 90);
        assert!(ms.open());
        ms.close();
        let log = run(&mut ms, Vec::new(), 1);
        let log2 = run(&mut ms, vec![(APT_START, 0)], 90);
        assert!(ms.open(), "{log:?} {log2:?}");
        run(&mut ms, vec![(APT_START, 0)], 90);
        let level_keys: Vec<&str> = ms.stage.levels().iter().map(|l| l.key.as_str()).collect();
        eprintln!("levels after close: {level_keys:?}; {}", ms.object_summary());
    }

    /// Extras on the lean set: the pause item opens FE 18 with its rows; A on the cheat row asks the game for text
    /// (FE 46), `zombie` turns the cheat on with its dialog, again turns it off; an invalid character warns and asks
    /// again; Licenses opens the text viewer with this project's licenses; B closes the viewer, then Extras.
    /// Data-gated like the pause test.
    /// SpawnSkater on the lean set: Call Skater opens FE 62 with the pros, A calls one, A on another shows the limit
    /// popup (the Vita's 1), X on the called row dismisses it; Free Play opens it with You, X opens the Free Play options
    /// (FE 34), Right steps Traffic and B writes the values back.
    #[test]
    fn spawn_skater_calls_and_dismisses_and_opens_the_free_play_options() {
        use skate_core::spawn_skater::{Action, Skater};
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        if !movies.movies.contains_key(&movie_key(SPAWN_SKATER_PATH)) || !movies.movies.contains_key(&movie_key(FREE_SKATE_PATH)) {
            eprintln!("skipped: no SpawnSkater / Free Play options movie in this set");
            return;
        }
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        ms.menus.context.mode = "FreePlay".into();
        let pro = |k: &str| crate::fe::spawn_skater_host::Callable { key: k.into(), name_id: format!("ID_COMMON_{}", k.to_uppercase()) };
        ms.set_callable(vec![pro("chris_cole"), pro("danny_way")], 1);
        ms.set_free_play(skate_core::living_world::FreePlay { traffic: 0.2, pedestrians: 0.0, ai_skaters: true });
        let run = |ms: &mut MenuStage, keys: Vec<(u32, usize)>, frames: usize| {
            let mut frame = ms.update_keys(keys, 1.0 / 60.0);
            for _ in 1..frames {
                let mut next = ms.update_keys(Vec::new(), 1.0 / 60.0);
                frame.log.append(&mut next.log);
                frame.spawn_skater.append(&mut next.spawn_skater);
                frame.free_play = frame.free_play.or(next.free_play);
            }
            assert!(!ms.failed, "{:?}", frame.log);
            frame
        };
        let open_item = |ms: &mut MenuStage, item: &str| {
            let view = front_end(&ms.menus, &ms.language, &ms.sub_options, ms.crossbar.clone()).view;
            let (tab, row) = (0..8)
                .flat_map(|t| (0..16).map(move |r| (t, r)))
                .find(|&(t, r)| view.item(t, r) == Some(item))
                .unwrap_or_else(|| panic!("{item} in the pause menu"));
            for _ in 0..8 {
                if ms.crossbar.menu == tab as i32 {
                    break;
                }
                run(ms, vec![(APT_RIGHT, 0)], 20);
            }
            for _ in 0..16 {
                if ms.crossbar.option == row as i32 {
                    break;
                }
                run(ms, vec![(13, 0)], 10);
            }
            run(ms, vec![(APT_NEXT, 0)], 90)
        };
        run(&mut ms, vec![(APT_START, 0)], 90);
        let frame = open_item(&mut ms, "SkateWith");
        assert!(frame.log.iter().any(|l| l.starts_with("open SpawnSkater") && l.ends_with("mode 100 rows 3")), "{:?}", frame.log);
        eprintln!("spawn skater natives {:?}", ms.stage.unhandled());
        // The picker starts on the first pickable row (the header is greyed): A calls Chris Cole.
        let frame = run(&mut ms, vec![(APT_NEXT, 0)], 30);
        assert_eq!(frame.spawn_skater, [Action::Call(Skater::Character("chris_cole".into()))], "{:?}", frame.log);
        // Down, A: the limit popup; A closes it.
        run(&mut ms, vec![(13, 0)], 10);
        let frame = run(&mut ms, vec![(APT_NEXT, 0)], 30);
        assert!(frame.spawn_skater.is_empty() && ms.popup_shown(), "{:?}", frame.log);
        assert!(ms.popups.shown().is_some_and(|(_, r)| r.description.starts_with("#There is already a skater called.")), "the Vita's cap of 1");
        run(&mut ms, vec![(APT_NEXT, 0)], 30);
        assert!(!ms.popup_shown());
        // Up, X: Chris Cole dismissed. B closes FE 62.
        run(&mut ms, vec![(12, 0)], 10);
        let frame = run(&mut ms, vec![(APT_MISC, 0)], 30);
        assert_eq!(frame.spawn_skater, [Action::Dismiss(Skater::Character("chris_cole".into()))], "{:?}", frame.log);
        let frame = run(&mut ms, vec![(APT_PREVIOUS, 0)], 60);
        assert!(frame.log.iter().any(|l| l.starts_with("close SpawnSkater")), "{:?}", frame.log);
        // Free Play: FE 62 with You; X opens FE 34; Right steps Traffic 0.2 -> 0.3; B writes it back.
        let frame = open_item(&mut ms, "SoloFreeskateOptions");
        assert!(frame.log.iter().any(|l| l.starts_with("open SpawnSkater") && l.ends_with("mode 200 rows 2")), "{:?}", frame.log);
        let frame = run(&mut ms, vec![(APT_MISC, 0)], 90);
        assert!(frame.log.iter().any(|l| l.starts_with("open Free Play options")), "{:?}", frame.log);
        eprintln!("free play natives {:?}", ms.stage.unhandled());
        run(&mut ms, vec![(APT_RIGHT, 0)], 10);
        let frame = run(&mut ms, vec![(APT_PREVIOUS, 0)], 60);
        let values = frame.free_play.expect("B writes the values back");
        assert!((values.traffic - 0.3).abs() < 1e-6 && values.ai_skaters, "{values:?}");
        assert!(frame.log.iter().any(|l| l.starts_with("close Free Play options")), "{:?}", frame.log);
    }

    /// The loading screen on the lean set: a load fades to black, pushes FE 52, drops the black when it is up and
    /// shows a panel (Coach Frank or a hint); the load's end fades to black, closes it and fades back in.
    #[test]
    fn the_loading_screen_runs_around_a_load() {
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        // The lines as vita-bake writes them (from the collections).
        let tips = skate_data::collections::Collections::load(&assets).map(|c| skate_data::tips_loader::read(&c)).unwrap_or_default();
        let tips = serde_json::to_vec(&tips).unwrap();
        let read = |p: &str| if p == TIPS_LOADER_DATA { Ok(tips.clone()) } else { std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}")) };
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        if !movies.movies.contains_key(&movie_key(TIPS_LOADER_PATH)) {
            eprintln!("skipped: no loading screen movie in this set");
            return;
        }
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        assert!(ms.has_loading_screen(), "movie, template and lines");
        let mut log = Vec::new();
        let run = |ms: &mut MenuStage, log: &mut Vec<String>, frames: usize| {
            for _ in 0..frames {
                let mut f = ms.update_keys(Vec::new(), 1.0 / 60.0);
                log.append(&mut f.log);
                assert!(!ms.failed, "{log:?}");
            }
        };
        // Career stats for the Stats panel (rows only with the tables' career stats table).
        let mut stats = skate_core::career_stats::CareerStats::default();
        stats.record(skate_core::career_stats::row::DISTANCE, 1234.5);
        stats.record(skate_core::career_stats::row::SUCCESSFUL_TRICKS, 42.0);
        ms.set_career_stats(&stats, false);
        ms.set_loading(true);
        run(&mut ms, &mut log, 120);
        assert!(ms.load_covered() && ms.fade.alpha == 0.0, "up, the black dropped: {log:?}");
        assert!(log.iter().any(|l| l.starts_with("open loading screen")), "{log:?}");
        assert!(!log.iter().any(|l| l.starts_with("_global.Load_")), "the autosave wheel: {log:?}");
        assert_eq!(ms.tips_loader.stats.is_empty(), ms.career_table.is_empty(), "Stats rows with the table");
        eprintln!("stats rows {:?}", ms.tips_loader.stats);
        let draws = ms.draw();
        eprintln!("loading screen draws {}; natives {:?}", draws.len(), ms.stage.unhandled());
        assert!(!draws.is_empty());
        // Ten seconds: the panel goes out and the next comes in.
        run(&mut ms, &mut log, 600);
        ms.set_loading(false);
        run(&mut ms, &mut log, 120);
        assert!(log.iter().any(|l| l.starts_with("close loading screen")), "{log:?}");
        assert!(!ms.load_active() && ms.fade.alpha == 0.0, "faded back in: {log:?}");
    }

    #[test]
    fn extras_enters_a_cheat_and_shows_the_licenses() {
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        if !movies.movies.contains_key(&movie_key(EXTRAS_PATH)) {
            eprintln!("skipped: no Extras movie in this set");
            return;
        }
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        ms.menus.context.mode = "FreePlay".into();
        let run = |ms: &mut MenuStage, keys: Vec<(u32, usize)>, frames: usize| {
            let mut frame = ms.update_keys(keys, 1.0 / 60.0);
            for _ in 1..frames {
                let mut next = ms.update_keys(Vec::new(), 1.0 / 60.0);
                frame.log.append(&mut next.log);
                frame.cheats.append(&mut next.cheats);
                frame.text_entry = frame.text_entry.or(next.text_entry);
            }
            assert!(!ms.failed, "{:?}", frame.log);
            frame
        };
        run(&mut ms, vec![(APT_START, 0)], 90);
        let view = front_end(&ms.menus, &ms.language, &ms.sub_options, ms.crossbar.clone()).view;
        let (tab, row) = (0..8)
            .flat_map(|t| (0..16).map(move |r| (t, r)))
            .find(|&(t, r)| view.item(t, r) == Some(EXTRAS_ITEM))
            .expect("Extras in the pause menu");
        for _ in 0..8 {
            if ms.crossbar.menu == tab as i32 {
                break;
            }
            run(&mut ms, vec![(APT_RIGHT, 0)], 20);
        }
        for _ in 0..16 {
            if ms.crossbar.option == row as i32 {
                break;
            }
            run(&mut ms, vec![(13, 0)], 10);
        }
        let frame = run(&mut ms, vec![(APT_NEXT, 0)], 90);
        assert!(frame.log.iter().any(|l| l.starts_with("open Extras")), "{:?}", frame.log);
        let draws = ms.draw();
        eprintln!("extras draws {}; natives {:?}", draws.len(), ms.stage.unhandled());
        // Row 0: the cheat code.
        let frame = run(&mut ms, vec![(APT_NEXT, 0)], 30);
        let (title, request) = frame.text_entry.expect("the keyboard is asked for");
        assert_eq!((title.as_str(), request.callback.as_str()), ("Enter Cheat Code", skate_core::text_entry::CHEAT_CALLBACK));
        ms.text_entered(Some("zombie".into()));
        let frame = run(&mut ms, Vec::new(), 30);
        assert_eq!(frame.cheats, vec![("zombie".to_owned(), true)], "{:?}", frame.log);
        assert!(ms.popup_shown(), "the cheat's dialog");
        let frame = run(&mut ms, vec![(APT_NEXT, 0)], 30);
        assert!(!ms.popup_shown(), "{:?} ready {}", frame.log, ms.popup.input_ready);
        // Again: off. Then a character outside the set: the warning, and the keyboard again after OK.
        run(&mut ms, vec![(APT_NEXT, 0)], 30);
        ms.text_entered(Some("zombie".into()));
        assert_eq!(run(&mut ms, Vec::new(), 30).cheats, vec![("zombie".to_owned(), false)]);
        run(&mut ms, vec![(APT_NEXT, 0)], 30);
        run(&mut ms, vec![(APT_NEXT, 0)], 30);
        ms.text_entered(Some("zombié".into()));
        let frame = run(&mut ms, Vec::new(), 30);
        assert!(ms.popup_shown() && frame.cheats.is_empty() && frame.text_entry.is_none());
        let frame = run(&mut ms, vec![(APT_NEXT, 0)], 30);
        assert!(frame.text_entry.is_some(), "asked again after the warning");
        ms.text_entered(None);
        let frame = run(&mut ms, Vec::new(), 30);
        assert!(frame.log.iter().any(|l| l.starts_with("close text entry")), "{:?}", frame.log);
        // Down row by row to Licenses (the picker skips the greyed Credits), A on each: rows 1-3 say the code worked
        // (OK closes it), Licenses opens the viewer with the licenses.
        let mut frame = Frame::default();
        for _ in 0..8 {
            run(&mut ms, vec![(13, 0)], 10);
            frame = run(&mut ms, vec![(APT_NEXT, 0)], 60);
            if frame.log.iter().any(|l| l.starts_with("open text viewer")) {
                break;
            }
            if ms.popup_shown() {
                run(&mut ms, vec![(APT_NEXT, 0)], 30);
            }
        }
        assert!(frame.log.iter().any(|l| l.starts_with("open text viewer")), "{:?}", frame.log);
        assert!(ms.tos.viewer.as_ref().is_some_and(|v| v.words.len() > 100));
        // The movie's line fields (`eval("mcTOSText.txtBox" + i)`) hold the licenses' first lines.
        let vm = &ms.stage.vm;
        let obj = |v: Value| if let Value::Object(o) = v { Some(o) } else { None };
        let this = obj(vm.get(vm.global, "nucleus_tos")).and_then(|c| obj(vm.get(c, "mThis"))).expect("nucleus_tos.mThis");
        let lines = obj(vm.get(this, "ref_lines")).expect("ref_lines");
        let text = |i: usize| obj(vm.get(lines, &i.to_string())).map(|l| vm.get(l, "text").text());
        eprintln!("viewer lines: {:?}", (0..6).map(text).collect::<Vec<_>>());
        assert_eq!(text(0).as_deref(), Some("#Skate Vita"), "one entry per line");
        let draws = ms.draw();
        eprintln!("viewer draws {}; natives {:?}", draws.len(), ms.stage.unhandled());
        let frame = run(&mut ms, vec![(APT_PREVIOUS, 0)], 60);
        assert!(frame.log.iter().any(|l| l.starts_with("close text viewer")), "{:?}", frame.log);
        let frame = run(&mut ms, vec![(APT_PREVIOUS, 0)], 60);
        assert!(frame.log.iter().any(|l| l.starts_with("close Extras")), "{:?}", frame.log);
    }

    /// The Skate Profile on the lean set: the pause item opens FE 61 on tab 0, the tab key steps it (the movie asks
    /// the Progression / Stats objects), B closes it. Data-gated like the pause test.
    #[test]
    fn skate_profile_opens_steps_its_tab_and_closes() {
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        if !movies.movies.contains_key(&movie_key(OFFLINE_PROFILE_PATH)) {
            eprintln!("skipped: no Skate Profile movie in this set");
            return;
        }
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let run = |ms: &mut MenuStage, keys: Vec<(u32, usize)>, frames: usize| {
            let mut frame = ms.update_keys(keys, 1.0 / 60.0);
            for _ in 1..frames {
                frame.log.append(&mut ms.update_keys(Vec::new(), 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{:?}", frame.log);
            frame
        };
        run(&mut ms, vec![(APT_START, 0)], 90);
        let view = front_end(&ms.menus, &ms.language, &ms.sub_options, ms.crossbar.clone()).view;
        let (tab, row) = (0..8)
            .flat_map(|t| (0..16).map(move |r| (t, r)))
            .find(|&(t, r)| view.item(t, r) == Some(OFFLINE_PROFILE_ITEM))
            .expect("the Skate Profile in the pause menu");
        for _ in 0..8 {
            if ms.crossbar.menu == tab as i32 {
                break;
            }
            run(&mut ms, vec![(APT_RIGHT, 0)], 20);
        }
        for _ in 0..16 {
            if ms.crossbar.option == row as i32 {
                break;
            }
            run(&mut ms, vec![(13, 0)], 10);
        }
        let frame = run(&mut ms, vec![(APT_NEXT, 0)], 90);
        assert!(frame.log.iter().any(|l| l.starts_with("open Skate Profile")), "{:?}", frame.log);
        assert!(ms.offline_profile.loaded && ms.offline_profile.screen.tab == 0);
        eprintln!("profile draws {}; natives {:?}", ms.draw().len(), ms.stage.unhandled());
        run(&mut ms, vec![(PROFILE_NEXT_TAB, 0)], 30);
        assert_eq!(ms.offline_profile.screen.tab, 1);
        eprintln!("tab 1 draws {}", ms.draw().len());
        let frame = run(&mut ms, vec![(APT_PREVIOUS, 0)], 60);
        assert!(frame.log.iter().any(|l| l.starts_with("close Skate Profile")), "{:?}", frame.log);
    }
}


#[cfg(test)]
mod replay_tests {
    use super::*;
    use crate::fe::replay_menu_host::{mode, ReplayScreenFeed};

    /// Park Setup (New Park) on the lean set: opened over the pause menu with lots, shown after its intro, every
    /// `ParkSetup_*` native answered and the thumbnail drawn from the exported images; the C++ keys move the rows, A takes
    /// the template, B goes back to the pause menu. Data-gated like the pause test.
    #[test]
    fn park_setup_opens_moves_takes_a_template_and_closes() {
        use crate::fe::park_setup_host::{ParkSetupChoice, ParkSetupLot};
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let thumbs = movies.has_named_image("fe\\source\\images\\skatepark\\starter_full");
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let lot = |name: &str| ParkSetupLot {
            name: name.into(),
            unlocked: true,
            templates: vec![("ID_FULL".into(), format!("{name}_full")), ("ID_EMPTY".into(), format!("{name}_empty"))],
        };
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            let mut log = Vec::new();
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                log.append(&mut ms.update_keys(keys, 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        run(&mut ms, vec![APT_START], 90);
        if !ms.open_park_setup(vec![lot("starter"), lot("industrial")]) {
            eprintln!("skipped: no Park Setup movie in this set");
            return;
        }
        let log = run(&mut ms, Vec::new(), 120);
        assert!(log.iter().any(|l| l.starts_with("open park setup")), "{log:?}");
        assert!(ms.park_setup.shown, "shown after its intro: {log:?}");
        let draws = ms.draw();
        eprintln!("park setup: {} draws, thumbnails exported {thumbs}, unhandled {:?}", draws.len(), ms.stage.unhandled());
        assert!(!ms.stage.unhandled().iter().any(|n| n.starts_with("SkatePark.")), "{:?}", ms.stage.unhandled());
        if thumbs {
            // The image slot's quad draws the slot's image itself.
            let shown = draws.iter().any(|d| d.texture == "image/fe/source/images/skatepark/starter_full");
            assert!(shown, "the thumbnail is drawn");
        }
        // Right: the next lot; Down, Right: its Empty template; A takes it. Each press as the Vita sends it: the pad's code
        // and its d-pad twin, one step.
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 5);
        run(&mut ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 5);
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 5);
        assert_eq!((ms.park_setup.lot, ms.park_setup.template, ms.park_setup.bar), (1, 1, 1));
        run(&mut ms, vec![APT_NEXT], 5);
        assert_eq!(ms.take_park_setup_choices(), vec![ParkSetupChoice { lot: 1, template: 1 }]);
        let log = run(&mut ms, vec![APT_PREVIOUS], 60);
        assert!(log.iter().any(|l| l.starts_with("close park setup")), "{log:?}");
        assert!(ms.open(), "B returns to the pause menu");
    }

    /// The character customisation on the lean set: opened on a skater, shown after its intro, every `CAC_*` native
    /// it asks answered; the movie moves the rows, Left and Right step them, A enters and leaves Edit Style, B exits.
    /// Data-gated like the pause test.
    #[test]
    fn the_customisation_opens_edits_a_skater_and_closes() {
        use skate_core::cac::{Cac, MENU_EDIT_SKATER, MENU_EDIT_STYLE};
        use skate_core::skater_record::SkaterRecord;
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            let mut log = Vec::new();
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                log.append(&mut ms.update_keys(keys, 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        run(&mut ms, vec![APT_START], 90);
        let mut record = SkaterRecord::cleared();
        (record.trucks, record.wheels) = (0.7, 0.7);
        if !ms.open_cac(Cac::open(record, &skate_core::unlocks::UnlockStore::fresh_profile(), None), 1234) {
            eprintln!("skipped: no customisation movie in this set");
            return;
        }
        let log = run(&mut ms, Vec::new(), 180);
        assert!(log.iter().any(|l| l.starts_with("open the customisation")), "{log:?}");
        assert!(ms.cac.shown, "shown after its intro: {log:?}");
        let draws = ms.draw();
        let unhandled = ms.stage.unhandled();
        eprintln!("customisation: {} draws, unhandled {unhandled:?}", draws.len());
        assert!(!unhandled.iter().any(|n| n.contains("CAC_")), "{unhandled:?}");
        assert!(draws.len() > 20, "the screen draws");
        let c = |ms: &MenuStage| ms.cac.screen.clone().unwrap();
        assert_eq!((c(&ms).menu(), c(&ms).cursor().0), (MENU_EDIT_SKATER, 0));
        // Three rows down to the trucks, as the Vita sends a press: the pad's code and its d-pad twin.
        for _ in 0..3 {
            run(&mut ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
        }
        assert_eq!(c(&ms).cursor().0, 3, "the movie moved the row");
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 6);
        assert!((c(&ms).record.trucks - 0.8).abs() < 1e-5, "{}", c(&ms).record.trucks);
        // Up to Edit Style; A enters it, the selection on its first row.
        run(&mut ms, vec![KEYS_UP[0], KEYS_UP[1]], 12);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!((c(&ms).menu(), c(&ms).cursor().0), (MENU_EDIT_STYLE, 0));
        // The style row (6th): Right turns Aggressive to Standard.
        for _ in 0..5 {
            run(&mut ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
        }
        assert_eq!(c(&ms).cursor().0, 5);
        assert_eq!(c(&ms).record.style, 3);
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 6);
        assert_eq!(c(&ms).record.style, 0);
        // A accepts, back on Edit Skater; B asks first (82603290), Yes exits with the record, back on the pause menu.
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_EDIT_SKATER);
        let log = run(&mut ms, vec![APT_PREVIOUS], 60);
        assert!(ms.popup_shown() && log.iter().any(|l| l.contains("cac.exit")), "{log:?}");
        let log = run(&mut ms, vec![APT_NEXT], 60);
        assert!(log.iter().any(|l| l.starts_with("close the customisation")), "{log:?}");
        let finished = ms.take_cac().expect("the record comes back");
        assert_eq!(finished.style, 0);
        assert!((finished.trucks - 0.8).abs() < 1e-5);
        assert!(ms.take_cac().is_none());
        assert!(ms.open(), "B returns to the pause menu");
    }

    /// Slappy's on the real movie with a small catalog: Edit Skater > Slappy's > T-shirts opens the thumbnails, the
    /// tiles load their pictures through the image slots, a highlight tries the tee on after half a second, A wears it
    /// and goes back to Slappy's, B leaves. Data-gated like the pause test.
    #[test]
    fn slappys_lists_thumbnails_tries_on_and_wears() {
        use skate_core::cac::{Cac, CacData, MENU_SLAPPYS, MENU_THUMBNAILS};
        use skate_core::cas_rules::{Catalog, Vault};
        use skate_core::skater_record::{Item, SkaterRecord, Slot};
        use std::collections::BTreeMap;
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            let mut log = Vec::new();
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                log.append(&mut ms.update_keys(keys, 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        run(&mut ms, vec![APT_START], 90);
        let tee = |n: u64| Item { model: 0x70 + n, material: 1 };
        let row = |n: u64, brand: &str| {
            let fields: BTreeMap<String, String> = [("Gender", "male"), ("TopType", "tshirt"), ("ManufacturerID", brand), ("ModelName", "tee"), ("MaterialVariantName", &format!("v{n}"))]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            (Slot::OuterTorso, tee(n), fields)
        };
        let catalog = Catalog::build(&[row(0, "zoo"), row(1, "alien"), row(2, "volcom")], &[]);
        let data = std::sync::Arc::new(CacData { catalog, vault: Vault::default(), fallback: [SkaterRecord::cleared(), SkaterRecord::cleared()] });
        let mut record = SkaterRecord::cleared();
        record.worn[Slot::OuterTorso as usize] = tee(0);
        if !ms.open_cac(Cac::open(record, &skate_core::unlocks::UnlockStore::fresh_profile(), Some(data)), 0) {
            eprintln!("skipped: no customisation movie in this set");
            return;
        }
        run(&mut ms, Vec::new(), 180);
        let c = |ms: &MenuStage| ms.cac.screen.clone().unwrap();
        // Slappy's (row 1), then T-shirts (its row 1).
        run(&mut ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_SLAPPYS);
        run(&mut ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!((c(&ms).menu(), c(&ms).row_count()), (MENU_THUMBNAILS, 4), "remove, the worn tee, then by brand");
        let unhandled = ms.stage.unhandled();
        assert!(!unhandled.iter().any(|n| n.contains("CAC_")), "{unhandled:?}");
        // The 3 x 3 grid loads its four tiles' pictures (LoadRX2Texture > ImageRenderer_Load).
        let images: Vec<String> = ms.image_slots.slots.iter().filter_map(|s| s.image.clone()).collect();
        assert!(images.iter().any(|i| i.ends_with("fe/cas/general/remove")), "{images:?}");
        assert!(images.iter().any(|i| i.ends_with("fe/cas/male/outertorso/tee_v1")), "{images:?}");
        // The tiles are drawn with those keys (the menu set has no such images; the game supplies them).
        let drawn: Vec<String> = ms.draw().into_iter().map(|d| d.texture).filter(|t| t.starts_with(CAS_THUMB_PREFIX)).collect();
        assert!(drawn.iter().any(|t| t.ends_with("fe/cas/general/remove")) && drawn.iter().any(|t| t.ends_with("tee_v1")), "{drawn:?}");
        // Right to the alien tee (row 2): tried on after half a second, the record unchanged.
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 6);
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 40);
        assert_eq!(c(&ms).trying, Some((Slot::OuterTorso, tee(1))));
        assert_eq!(ms.cac_look().map(|(r, _)| r.worn[Slot::OuterTorso as usize]), Some(tee(1)));
        assert_eq!(c(&ms).record.worn[Slot::OuterTorso as usize], tee(0));
        ms.cac_look_done();
        // A wears it and goes back to Slappy's.
        run(&mut ms, vec![APT_NEXT], 30);
        ms.cac_look_done();
        assert_eq!((c(&ms).menu(), c(&ms).record.worn[Slot::OuterTorso as usize], c(&ms).trying), (MENU_SLAPPYS, tee(1), None));
        // B to Edit Skater, B asks, Yes closes.
        run(&mut ms, vec![APT_PREVIOUS], 30);
        run(&mut ms, vec![APT_PREVIOUS], 30);
        let log = run(&mut ms, vec![APT_NEXT], 60);
        assert!(log.iter().any(|l| l.starts_with("close the customisation")), "{log:?}");
        assert_eq!(ms.take_cac().map(|r| r.worn[Slot::OuterTorso as usize]), Some(tee(1)));
    }

    /// The tattoo pages on the real movie: Edit Skater > Back Alley Pete's > Upper Body Tattoo > Front > the first
    /// category > a stamp opens the placement page; the movie asks for nothing the host lacks, the keys and the sticks
    /// move the placement, A goes back to Pete's. Data-gated like the pause test.
    #[test]
    fn the_tattoo_pages_open_on_the_real_movie_and_place_a_stamp() {
        use skate_core::cac::{Cac, CacData, MENU_BACK_ALLEY, MENU_TATTOO_ADJUST_UPPER, MENU_TATTOO_CATEGORIES, MENU_TATTOO_STAMPS, MENU_TATTOO_UPPER};
        use skate_core::cas_rules::{Catalog, TattooCategory, Vault};
        use skate_core::skater_record::{Item, SkaterRecord, Slot};
        use std::collections::BTreeMap;
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            let mut log = Vec::new();
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                log.append(&mut ms.update_keys(keys, 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        run(&mut ms, vec![APT_START], 90);
        let stamp = |n: u64, category: &str| {
            let mut fields: BTreeMap<String, String> =
                [("ModelName", "logo_object"), ("MaterialVariantName", &format!("tattoo_{n}_mat"))].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            if !category.is_empty() {
                fields.insert("TattooCategory".into(), category.into());
            }
            (Slot::Misc, Item { model: 0xC0 + n, material: 1 }, fields)
        };
        let catalog = Catalog::build(&[stamp(1, "1"), stamp(2, "1"), stamp(3, "2")], &[]);
        let vault = Vault {
            tattoo_categories: vec![TattooCategory { text: "1".into(), untagged: false }, TattooCategory { text: "2".into(), untagged: false }],
            ..Vault::default()
        };
        let data = std::sync::Arc::new(CacData { catalog, vault, fallback: [SkaterRecord::cleared(), SkaterRecord::cleared()] });
        if !ms.open_cac(Cac::open(SkaterRecord::cleared(), &skate_core::unlocks::UnlockStore::fresh_profile(), Some(data)), 0) {
            eprintln!("skipped: no customisation movie in this set");
            return;
        }
        run(&mut ms, Vec::new(), 180);
        let c = |ms: &MenuStage| ms.cac.screen.clone().unwrap();
        let down = |ms: &mut MenuStage, n: usize| {
            for _ in 0..n {
                run(ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
            }
        };
        // Pete's (row 0); the upper tattoo is its row 7.
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_BACK_ALLEY);
        down(&mut ms, 7);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!((c(&ms).menu(), c(&ms).row_count()), (MENU_TATTOO_UPPER, 4));
        // The front: no tattoo yet, so the categories open.
        run(&mut ms, vec![APT_NEXT], 30);
        ms.cac_look_done();
        assert_eq!((c(&ms).menu(), c(&ms).row_count()), (MENU_TATTOO_CATEGORIES, 2));
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!((c(&ms).menu(), c(&ms).row_count()), (MENU_TATTOO_STAMPS, 3), "remove and the category's two stamps");
        let images: Vec<String> = ms.image_slots.slots.iter().filter_map(|s| s.image.clone()).collect();
        assert!(images.iter().any(|i| i.ends_with("fe/cas/general/remove")) && images.iter().any(|i| i.ends_with("logo_object_tattoo_2_mat")), "{images:?}");
        // The second stamp (row 2), then A opens the placement page.
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 12);
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 12);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_TATTOO_ADJUST_UPPER);
        assert!(ms.cac_look().is_some(), "the stamp asks for the mesh");
        ms.cac_look_done();
        let unhandled = ms.stage.unhandled();
        assert!(!unhandled.iter().any(|n| n.contains("CAC_")), "{unhandled:?}");
        // The sticks move it, X starts over, A goes back to Pete's.
        let start = c(&ms).record.adjust.values[1];
        let mut pad = [0.0f32; 24];
        pad[20] = 1.0;
        for _ in 0..10 {
            ms.update(&[pad], 1.0 / 60.0);
        }
        assert_ne!(c(&ms).record.adjust.values[1], start);
        ms.update(&[[0.0f32; 24]], 1.0 / 60.0);
        run(&mut ms, vec![APT_OPTION], 4);
        assert_eq!(c(&ms).record.adjust.values[1], start);
        run(&mut ms, vec![APT_NEXT], 30);
        ms.cac_look_done();
        assert_eq!(c(&ms).menu(), MENU_BACK_ALLEY);
        assert_eq!(c(&ms).record.graphics.len(), 1);
    }

    /// A deck's graphic on the real movie: Slappy's > Skateboards > Boards > a logoable deck > Add Graphic > Brand > a
    /// logo opens the board's placement page; the movie asks for nothing the host lacks. Data-gated like the others.
    #[test]
    fn a_decks_graphic_pages_open_on_the_real_movie() {
        use skate_core::cac::{Cac, CacData, MENU_ADJUST_BOARD, MENU_GRAPHIC_BRAND, MENU_GRAPHIC_CATEGORIES, MENU_SLAPPYS, MENU_THUMBNAILS, MENU_WEAR_STYLES};
        use skate_core::cas_rules::{Catalog, Vault};
        use skate_core::skater_record::{Item, SkaterRecord, Slot};
        use std::collections::BTreeMap;
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                ms.update_keys(keys, 1.0 / 60.0);
            }
            assert!(!ms.failed);
        };
        run(&mut ms, vec![APT_START], 90);
        let fields = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        let deck = Item { model: 0xB1, material: 1 };
        let logo = Item { model: 0xC0, material: 1 };
        let rows = vec![
            (Slot::SkateBoard, deck, fields(&[("IsLogoable", "1")])),
            // A text the catalog lacks reads as the empty field (82DDD7D8), so the genders must be held.
            (Slot::Misc, logo, fields(&[("ModelName", "logo_object"), ("MaterialVariantName", "logo_thing"), ("Gender", "female")])),
        ];
        let data = std::sync::Arc::new(CacData { catalog: Catalog::build(&rows, &[]), vault: Vault::default(), fallback: [SkaterRecord::cleared(), SkaterRecord::cleared()] });
        if !ms.open_cac(Cac::open(SkaterRecord::cleared(), &skate_core::unlocks::UnlockStore::fresh_profile(), Some(data)), 0) {
            eprintln!("skipped: no customisation movie in this set");
            return;
        }
        run(&mut ms, Vec::new(), 180);
        let c = |ms: &MenuStage| ms.cac.screen.clone().unwrap();
        let down = |ms: &mut MenuStage, n: usize| {
            for _ in 0..n {
                run(ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
            }
        };
        // Slappy's (row 1), Skateboards (its row 10 = the last), Boards (row 0).
        down(&mut ms, 1);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_SLAPPYS);
        down(&mut ms, 10);
        run(&mut ms, vec![APT_NEXT], 30);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!((c(&ms).menu(), c(&ms).title(), c(&ms).row_count()), (MENU_THUMBNAILS, "ID_CAC_BOARDS_TITLE", 1));
        run(&mut ms, vec![APT_NEXT], 30);
        ms.cac_look_done();
        assert_eq!((c(&ms).menu(), c(&ms).title(), c(&ms).busy), (MENU_WEAR_STYLES, "ID_CUSTOM_EDITDECK", false));
        // Add Graphic (row 0), Brand (row 0), the logo.
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_GRAPHIC_CATEGORIES);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!((c(&ms).menu(), c(&ms).row_count()), (MENU_GRAPHIC_BRAND, 1));
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_ADJUST_BOARD);
        assert!(ms.cac_look().is_some());
        ms.cac_look_done();
        let unhandled = ms.stage.unhandled();
        assert!(!unhandled.iter().any(|n| n.contains("CAC_")), "{unhandled:?}");
        // A goes back to Slappy's.
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_SLAPPYS);
        assert_eq!(c(&ms).record.graphics.len(), 1);
    }

    /// Wear Styles on the real movie: a colourisable tee picked on the thumbnails opens the morph page with its colour
    /// row (swatches from the outer torso's zone); Right steps the colour onto the tee, A accepts back to Slappy's.
    #[test]
    fn wear_styles_opens_on_a_colourisable_item_and_its_colour_row_tints_it() {
        use skate_core::cac::{Cac, CacData, MENU_SLAPPYS, MENU_WEAR_STYLES};
        use skate_core::cas_rules::{Catalog, Colour, Vault, Zone};
        use skate_core::skater_record::{Item, SkaterRecord, Slot};
        use std::collections::BTreeMap;
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            let mut log = Vec::new();
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                log.append(&mut ms.update_keys(keys, 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        run(&mut ms, vec![APT_START], 90);
        let tee = Item { model: 0x70, material: 1 };
        let fields: BTreeMap<String, String> =
            [("Gender", "male"), ("TopType", "tshirt"), ("IsColourizable", "1"), ("ModelName", "tee"), ("MaterialVariantName", "plain")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let catalog = Catalog::build(&[(Slot::OuterTorso, tee, fields)], &[]);
        let colour = |key: u64, tag: &str, v: f32| Colour { key, primary: [v; 3], swatch: [v; 3], secondary: [0.0; 3], tag: tag.into() };
        let vault = Vault { zones: vec![Zone { key: 9, slot: 9, colours: vec![colour(1, "red", 0.7), colour(2, "blue", 0.2)] }], ..Vault::default() };
        let data = std::sync::Arc::new(CacData { catalog, vault, fallback: [SkaterRecord::cleared(), SkaterRecord::cleared()] });
        if !ms.open_cac(Cac::open(SkaterRecord::cleared(), &skate_core::unlocks::UnlockStore::fresh_profile(), Some(data)), 0) {
            eprintln!("skipped: no customisation movie in this set");
            return;
        }
        run(&mut ms, Vec::new(), 180);
        let c = |ms: &MenuStage| ms.cac.screen.clone().unwrap();
        run(&mut ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
        run(&mut ms, vec![APT_NEXT], 30);
        run(&mut ms, vec![KEYS_DOWN[0], KEYS_DOWN[1]], 12);
        run(&mut ms, vec![APT_NEXT], 30);
        // Tiles: remove, the tee. Right to the tee, A.
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 12);
        run(&mut ms, vec![APT_NEXT], 30);
        ms.cac_look_done();
        assert_eq!((c(&ms).menu(), c(&ms).row_count(), c(&ms).record.worn[Slot::OuterTorso as usize]), (MENU_WEAR_STYLES, 1, tee));
        let unhandled = ms.stage.unhandled();
        assert!(!unhandled.iter().any(|n| n.contains("CAC_")), "{unhandled:?}");
        // Right on the colour row: the second colour on the tee's layer.
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 12);
        ms.cac_look_done();
        assert!(c(&ms).record.colours.iter().any(|l| l.index == 9 && l.part == tee && l.primary == [0.2; 3]), "{:?}", c(&ms).record.colours);
        run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(c(&ms).menu(), MENU_SLAPPYS);
    }

    /// The team screen (FE 67) on the real movie: shown after its intro with the team's data, every TeamManagement
    /// native answered, the empty-name warning raised; Y opens the logos, B comes back then closes. Data-gated.
    #[test]
    fn the_team_screen_opens_shows_the_team_and_closes() {
        use skate_core::team::{Team, STATE_LOGOS};
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            let mut log = Vec::new();
            let mut popups = Vec::new();
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                let f = ms.update_keys(keys, 1.0 / 60.0);
                log.extend(f.log);
                popups.extend(f.team_effects);
            }
            assert!(!ms.failed, "{log:?}");
            (log, popups)
        };
        run(&mut ms, vec![APT_START], 90);
        if !ms.open_team(Team::open(String::new(), vec!["Me".into()])) {
            eprintln!("skipped: no team movie in this set");
            return;
        }
        let (log, _) = run(&mut ms, Vec::new(), 180);
        assert!(log.iter().any(|l| l.starts_with("open the team screen")), "{log:?}");
        assert!(ms.team.shown, "shown after its intro: {log:?}");
        let draws = ms.draw().len();
        let unhandled = ms.stage.unhandled();
        eprintln!("team screen: {draws} draws, unhandled {unhandled:?}");
        assert!(!unhandled.iter().any(|n| n.contains("TeamManagement")), "{unhandled:?}");
        assert!(ms.popup_shown(), "the empty-name warning");
        assert!(ms.team.asked > 0, "the movie read the team's data");
        // OK on the warning asks for the team's name.
        let (_, effects) = run(&mut ms, vec![APT_NEXT], 60);
        assert_eq!(effects, [skate_core::team::Effect::RenameTeam]);
        ms.team_named(None, "Rippers");
        assert_eq!(ms.team().unwrap().name, "Rippers");
        run(&mut ms, Vec::new(), 30);
        // A on the first option: Edit Skater on the player.
        let (_, effects) = run(&mut ms, vec![APT_NEXT], 30);
        assert_eq!(effects, [skate_core::team::Effect::EditSkater(0)]);
        // The customisation over it, then B: the stack opens the team screen again, with its team, shown and keyed.
        assert!(ms.open_cac(skate_core::cac::Cac::open(skate_core::skater_record::SkaterRecord::cleared(), &skate_core::unlocks::UnlockStore::fresh_profile(), None), 0));
        let (log, _) = run(&mut ms, Vec::new(), 120);
        assert!(log.iter().any(|l| l.starts_with("open the customisation")), "{log:?}");
        run(&mut ms, vec![APT_PREVIOUS], 60);
        assert!(ms.popup_shown(), "the exit question");
        let (log, _) = run(&mut ms, vec![APT_NEXT], 180);
        assert!(log.iter().any(|l| l.starts_with("open the team screen")), "{log:?}");
        assert!(ms.team.shown && ms.team().is_some_and(|t| t.name == "Rippers"), "{log:?}");
        // Y: the logos; B: back on the members; B: closed.
        run(&mut ms, vec![APT_OPTION], 30);
        assert!(ms.team().unwrap().options().is_empty(), "no options on the logos");
        run(&mut ms, vec![APT_PREVIOUS], 30);
        assert_eq!(ms.team().unwrap().options().len(), 3);
        let (log, _) = run(&mut ms, vec![APT_PREVIOUS], 60);
        assert!(log.iter().any(|l| l.starts_with("close the team screen")), "{log:?}");
        let _ = STATE_LOGOS;
    }

    /// The browser on parks (My Parks) on the lean set: opened with saved parks, shown after its intro, every
    /// CommonBrowser native it asks answered; the d-pad and the tab shoulders move it, A selects the park, B closes it.
    /// Data-gated like the pause test.
    #[test]
    fn the_park_browser_opens_moves_selects_and_closes() {
        use crate::fe::browser_host::{BrowserEvent, BrowserItem};
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        let parks: Vec<BrowserItem> = (0..3).map(|i| BrowserItem { name: format!("park {i}"), date: "#2026-11-01".into(), picture: Some(i) }).collect();
        let run = |ms: &mut MenuStage, keys: Vec<u32>, frames: usize| {
            let mut log = Vec::new();
            for frame in 0..frames {
                let keys = if frame == 0 { keys.iter().map(|&k| (k, 0)).collect() } else { Vec::new() };
                log.append(&mut ms.update_keys(keys, 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        // Over the pause menu, as skate.Park > My Parks opens it.
        run(&mut ms, vec![APT_START], 90);
        assert!(ms.open());
        if !ms.open_browser(parks.clone()) {
            eprintln!("skipped: no browser movie in this set");
            return;
        }
        let log = run(&mut ms, Vec::new(), 120);
        assert!(log.iter().any(|l| l.starts_with("open browser")), "{log:?}");
        assert!(ms.browser.shown, "shown after its intro: {log:?}");
        eprintln!("browser: {} draws, unhandled {:?}", ms.draw().len(), ms.stage.unhandled());
        assert!(!ms.stage.unhandled().iter().any(|n| n.starts_with("CommonBrowser.")), "{:?}", ms.stage.unhandled());
        // Right twice (the last park), RB to Downloaded (empty) and back with LB: the selection starts over.
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 5);
        run(&mut ms, vec![KEYS_RIGHT[0], KEYS_RIGHT[1]], 5);
        assert_eq!(ms.browser.current, 2, "one step a press (the d-pad twin is not a second)");
        run(&mut ms, vec![KEY_RB], 10);
        assert_eq!((ms.browser.top, ms.browser.shown_items().len()), (1, 0));
        run(&mut ms, vec![KEY_LB], 10);
        assert_eq!((ms.browser.top, ms.browser.current), (0, 0));
        // A: the movie selects the park under the cursor.
        run(&mut ms, vec![KEYS_RIGHT[0]], 5);
        run(&mut ms, vec![APT_NEXT], 10);
        assert_eq!(ms.take_browser_events(), vec![BrowserEvent::Select(1)]);
        // A delete: the parks shown again, the cursor kept in range.
        ms.set_browser_items(parks[..1].to_vec());
        run(&mut ms, Vec::new(), 5);
        assert_eq!(ms.browser.current, 0);
        // A park written or deleted: the autosave layer's save wheel, then hidden.
        let before = ms.draw().len();
        ms.set_save_tag(true);
        let log = run(&mut ms, Vec::new(), 10);
        assert!(log.iter().any(|l| l == "save tag on") && !log.iter().any(|l| l.starts_with("_global.Save")), "{log:?}");
        assert!(ms.draw().len() > before, "the wheel draws");
        ms.set_save_tag(false);
        let log = run(&mut ms, Vec::new(), 2);
        assert!(log.iter().any(|l| l == "save tag off") && !ms.save_tag_active(), "{log:?}");
        let log = run(&mut ms, vec![APT_PREVIOUS], 60);
        assert!(log.iter().any(|l| l.starts_with("close browser")), "{log:?}");
        assert!(!ms.browser_open() && ms.open(), "B returns to the pause menu");
    }

    /// The Object Dropper screen on the lean set: opened with the editor's feed (park, then the open world), shown after
    /// its intro, the editor's screen modes posted (catalogue, items, manipulate), every SkatePark native it asks
    /// answered, the Feng Shui bars drawn; closed with the editor. Data-gated like the pause test.
    #[test]
    fn the_object_dropper_screen_opens_takes_the_editor_modes_and_draws_the_bars() {
        use crate::fe::object_dropper_host::ObjectDropperFeed;
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        if !ms.has_object_dropper() {
            eprintln!("skipped: no Object Dropper movie in this set");
            return;
        }
        let run = |ms: &mut MenuStage, frames: usize| {
            let mut log = Vec::new();
            for _ in 0..frames {
                log.append(&mut ms.update_keys(Vec::new(), 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        for world in [false, true] {
            let feed = ObjectDropperFeed {
                world,
                supers: vec!["terrain".into(), "props".into()],
                subs: vec![("street".into(), 2), ("vert".into(), 1)],
                items: vec!["aa_padStreet_md_University".into(), "ab_kicker_lrg_Downtown".into()],
                terrain: true,
                terrain_bar: [0.5, 0.2],
                props_bar: [0.8, 0.0],
                ..Default::default()
            };
            ms.set_object_dropper(Some(feed.clone()), &[0], &[]);
            let log = run(&mut ms, 90);
            assert!(log.iter().any(|l| l.starts_with("open ObjectDropper")), "{log:?}");
            assert!(ms.object_dropper.shown, "shown after its intro: {log:?}");
            // The catalogue, its item list, an item down, the size, then the item taken.
            let item = Some(("aa_padStreet_md_University".to_owned(), [2.0, 1.0, 4.0]));
            ms.set_object_dropper(Some(ObjectDropperFeed { item: item.clone(), ..feed.clone() }), &[1, 29, 27, 8, 2, 25, 13, 2, 19], &[]);
            run(&mut ms, 30);
            let draws = ms.draw();
            let bars = draws.iter().filter(|d| d.fill == crate::fe::apt_scene::Fill::Solid && d.multiply[..3] == [0.0, 1.0, 1.0]).count();
            let props = draws.iter().filter(|d| d.fill == crate::fe::apt_scene::Fill::Solid && d.multiply[..3] == [1.0, 1.0, 0.0]).count();
            eprintln!("world {world}: {} draws, FungShui quads {bars}, FungShuiDynamic quads {props}, unhandled {:?}", draws.len(), ms.stage.unhandled());
            assert!(!ms.stage.unhandled().iter().any(|n| n.starts_with("SkatePark.")), "{:?}", ms.stage.unhandled());
            assert_eq!((bars, props), (usize::from(!world), 1), "the open world hides the terrain bar");
            for native in ["GetSuperCategories", "GetItems", "GetItemsUnlocked", "GetCurrentItemSizeInfo"] {
                assert!(ms.object_dropper.seen.contains(native), "{native}: {:?}", ms.object_dropper.seen);
            }
            // B back to the sub category list.
            ms.set_object_dropper(Some(ObjectDropperFeed { selecting_sub: true, ..feed.clone() }), &[28], &[]);
            run(&mut ms, 5);
            assert!(ms.object_dropper.seen.contains("GetSubCategories") && ms.object_dropper.seen.contains("GetSubCategoryDescription"));
            ms.set_object_dropper(None, &[], &[]);
            let log = run(&mut ms, 60);
            assert!(log.iter().any(|l| l.starts_with("close ObjectDropper")), "{log:?}");
            assert!(!ms.object_dropper_loaded(), "the closed screen unloads after its outro");
        }
        // The Quick Menu over the park editor: opened, shown, its rows and the Options page asked, closed.
        if ms.object_quick_menu_id.is_some() {
            use crate::fe::object_dropper_host::QuickFeed;
            let park = ObjectDropperFeed { supers: vec!["terrain".into()], subs: vec![("street".into(), 2)], ..Default::default() };
            ms.set_object_dropper(Some(park.clone()), &[0], &[]);
            run(&mut ms, 90);
            // The colour grid is built as the menu loads, from the table then (red here).
            let quick = QuickFeed { rows: [true, true, true, true, false, false, true, false], snap: true, collision: true, cursor_speed: 1.0, colours: vec![[1.0, 0.0, 0.0]; 64], ..Default::default() };
            ms.object_dropper.seen.clear();
            ms.set_object_dropper(Some(ObjectDropperFeed { quick: Some(quick.clone()), ..park.clone() }), &[], &[]);
            let log = run(&mut ms, 90);
            assert!(log.iter().any(|l| l.starts_with("open ObjectQuickMenu")), "{log:?}");
            assert!(ms.object_dropper.quick_shown, "the Quick Menu shows after its intro: {log:?}");
            // Down to Options, A, Snapping off, B back to the list.
            ms.set_object_dropper(Some(ObjectDropperFeed { quick: Some(QuickFeed { snap: false, ..quick.clone() }), ..park.clone() }), &[], &[1, 2, 3, 8, 18, 13, 9, 3]);
            run(&mut ms, 10);
            let draws = ms.draw();
            eprintln!("quick menu: {} draws, seen {:?}, unhandled {:?}", draws.len(), ms.object_dropper.seen, ms.stage.unhandled());
            assert!(!ms.stage.unhandled().iter().any(|n| n.starts_with("SkatePark.")), "{:?}", ms.stage.unhandled());
            for native in ["QuickMenu_IsQuickMenuItemEnabled", "GetCurrentSnapState", "GetCurrentCursorSpeed"] {
                assert!(ms.object_dropper.seen.contains(native), "{native}: {:?}", ms.object_dropper.seen);
            }
            // The Style page: the movie's list takes the pad; Down highlights the next style for the editor.
            let style = QuickFeed { page: crate::fe::object_dropper_host::PAGE_STYLE, style_allowed: true, ..quick.clone() };
            ms.set_object_dropper(Some(ObjectDropperFeed { quick: Some(style.clone()), ..park.clone() }), &[], &[2, 11]);
            run(&mut ms, 10);
            ms.take_object_dropper_list_calls();
            let mut down = [0.0f32; 24];
            down[1] = 1.0;
            for frame in 0..12 {
                let pad = if frame < 6 { down } else { [0.0; 24] };
                ms.update(std::slice::from_ref(&pad), 1.0 / 60.0);
            }
            let calls = ms.take_object_dropper_list_calls();
            eprintln!("style list calls {calls:?}");
            assert!(calls.contains(&(1, false)), "Down highlights the second style: {calls:?}");
            // The colour grid: the swatches show the table's colours.
            let colour = QuickFeed { page: 2, ..quick.clone() };
            ms.set_object_dropper(Some(ObjectDropperFeed { quick: Some(colour), ..park.clone() }), &[], &[9, 0, 10, 23]);
            run(&mut ms, 10);
            let reds = ms.draw().iter().filter(|d| d.fill == crate::fe::apt_scene::Fill::Solid && d.multiply[..3] == [1.0, 0.0, 0.0]).count();
            assert!(reds >= 64, "the grid's swatches are drawn in their colours: {reds}");
            ms.set_object_dropper(Some(park.clone()), &[], &[]);
            let log = run(&mut ms, 60);
            assert!(log.iter().any(|l| l.starts_with("close ObjectQuickMenu")), "{log:?}");
            ms.set_object_dropper(None, &[], &[]);
            run(&mut ms, 60);
        }
        // The catalogue posted over and over in one update stays inside the VM's object budget.
        let feed = ObjectDropperFeed { supers: vec!["terrain".into(), "props".into()], subs: vec![("street".into(), 99)], items: (0..99).map(|i| format!("aa_pad{i}_md_University")).collect(), terrain: true, ..Default::default() };
        ms.set_object_dropper(Some(feed.clone()), &[0], &[]);
        run(&mut ms, 90);
        ms.set_object_dropper(Some(feed.clone()), &[1, 29, 28, 29, 4, 29, 28, 5, 29, 0, 1, 29, 28, 29, 4, 29, 28, 5, 29], &[]);
        run(&mut ms, 2);
        ms.set_object_dropper(None, &[], &[]);
        run(&mut ms, 60);
        // Closed and opened again at once (the outro still running): the new screen shows and takes its modes.
        let feed = ObjectDropperFeed { supers: vec!["props".into()], subs: vec![("dmo_ramps".into(), 2)], selecting_sub: true, ..Default::default() };
        ms.set_object_dropper(Some(feed.clone()), &[0], &[]);
        run(&mut ms, 90);
        ms.set_object_dropper(None, &[], &[]);
        run(&mut ms, 2);
        ms.set_object_dropper(Some(feed.clone()), &[0, 1], &[]);
        run(&mut ms, 90);
        assert!(ms.object_dropper.shown, "the reopened screen shows");
        ms.object_dropper.seen.clear();
        ms.set_object_dropper(Some(feed), &[28], &[]);
        run(&mut ms, 5);
        assert!(ms.object_dropper.seen.contains("GetSubCategories"), "{:?}", ms.object_dropper.seen);
        let levels = ms.stage.levels().iter().filter(|l| l.key == movie_key(OBJECT_DROPPER_PATH)).count();
        assert_eq!(levels, 1, "one dropper screen on the stage");
    }

    /// The instant replay screen on the lean set: opened with the replay's feed, shown after its intro, every
    /// FEReplayMenu native it asks answered; the timeline mode draws the strip (bands and keys); the help opens over
    /// it, turns a page and closes; the screen closes with the replay. Data-gated like the pause test.
    #[test]
    fn the_replay_screen_opens_shows_the_timeline_and_its_help() {
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        if read(TABLES).is_err() {
            eprintln!("skipped: no menu tables");
            return;
        }
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        assert!(ms.has_replay(), "Replay2 and its movie are in the set");
        let run = |ms: &mut MenuStage, frames: usize| {
            let mut log = Vec::new();
            for _ in 0..frames {
                log.append(&mut ms.update_keys(Vec::new(), 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        let mut feed = ReplayScreenFeed { active: true, instant: true, position: 0.9, playing: true, panel: true, length_seconds: 42, length_valid: true, mode: mode::INSTANT, ..Default::default() };
        ms.set_replay(Some(feed.clone()));
        let log = run(&mut ms, 90);
        assert!(log.iter().any(|l| l.starts_with("open Replay2")), "{log:?}");
        assert!(ms.replay_shown(), "shown after its intro: {log:?}");
        let draws = ms.draw();
        assert!(draws.len() > 5, "the replay screen draws: {}", draws.len());
        eprintln!("natives seen: {:?}; prompts {:?}", ms.replay_menu.seen, ms.prompts());
        // The timeline: bands and a key from the feed.
        feed.instant = false;
        feed.mode = mode::TIMELINE;
        feed.playing = false;
        feed.keys = vec![0.5];
        feed.segments = vec![(0.0, 0), (0.5, 3)];
        feed.head_fractions = (0.0, 1.0);
        ms.set_replay(Some(feed.clone()));
        run(&mut ms, 30);
        let draws = ms.draw();
        let keys = draws.iter().filter(|d| d.texture.ends_with("texture/key_default") || d.texture.ends_with("texture/key_glow")).count();
        eprintln!("timeline draws {} keys {keys}", draws.len());
        // The Finished menu as a popup over it: Down, A picks the second item; B cancels a cancelable one.
        let finished = |ms: &mut MenuStage| {
            ms.push_popup(PopupRequest {
                options: ["ID_REPLAYEDITOR_UPLOAD", "ID_REPLAYEDITOR_BASIC_SAVE_REPLAY", "ID_REPLAYEDITOR_EXIT"].map(String::from).to_vec(),
                cancelable: true,
                callback: "replay.finished".into(),
                ..Default::default()
            })
        };
        finished(&mut ms);
        run(&mut ms, 20);
        assert!(ms.popup_shown());
        ms.draw();
        eprintln!("popup prompts {:?}", ms.prompts());
        assert!(!ms.prompts().is_empty() && ms.prompts().iter().all(|(n, _)| matches!(n.as_str(), "A" | "B")), "only the popup's prompts");
        let mut choices = Vec::new();
        for keys in [vec![(13, 0)], vec![], vec![(APT_NEXT, 0)]] {
            choices.extend(ms.update_keys(keys, 1.0 / 60.0).popup_choices);
            for _ in 0..10 {
                choices.extend(ms.update_keys(Vec::new(), 1.0 / 60.0).popup_choices);
            }
        }
        assert_eq!(choices, vec![("replay.finished".to_owned(), 1)]);
        finished(&mut ms);
        run(&mut ms, 20);
        let mut choices = ms.update_keys(vec![(APT_PREVIOUS, 0)], 1.0 / 60.0).popup_choices;
        for _ in 0..10 {
            choices.extend(ms.update_keys(Vec::new(), 1.0 / 60.0).popup_choices);
        }
        // The popup movie's B answers -1 (the host's 255 is retail's constant for the same cancel).
        assert_eq!(choices.len(), 1);
        assert!(matches!(choices[0].1, -1 | skate_core::fe_popup::CHOICE_CANCEL), "{choices:?}");
        assert!(!ms.popup_shown());
        // The help: opens, turns a page, closes.
        feed.help = true;
        feed.help_mode = 1;
        ms.set_replay(Some(feed.clone()));
        let log = run(&mut ms, 60);
        assert!(log.iter().any(|l| l.starts_with("open ReplayHelp")), "{log:?}");
        assert!(ms.replay_help_shown(), "{log:?}");
        ms.draw();
        let help_prompts = ms.prompts().len();
        eprintln!("levels {:?}; help prompts {:?}", ms.stage.levels().iter().map(|l| l.key.as_str()).collect::<Vec<_>>(), ms.prompts());
        assert!(help_prompts > 0 && ms.prompts().iter().all(|(_, r)| r[1] > 0.0), "only the help's prompts");
        feed.help_turns = 1;
        ms.set_replay(Some(feed.clone()));
        run(&mut ms, 10);
        feed.help = false;
        ms.set_replay(Some(feed.clone()));
        let log = run(&mut ms, 30);
        assert!(log.iter().any(|l| l.starts_with("close ReplayHelp")), "{log:?}");
        // Exit from the Finished popup: the replay closes and the pause menu opens again (the game's Start).
        finished(&mut ms);
        run(&mut ms, 20);
        let mut log = ms.update_keys(vec![(13, 0)], 1.0 / 60.0).log;
        log.extend(run(&mut ms, 10));
        log.extend(ms.update_keys(vec![(13, 0)], 1.0 / 60.0).log);
        log.extend(run(&mut ms, 10));
        let frame = ms.update_keys(vec![(APT_NEXT, 0)], 1.0 / 60.0);
        log.extend(frame.log);
        let mut choices = frame.popup_choices;
        for _ in 0..5 {
            let f = ms.update_keys(Vec::new(), 1.0 / 60.0);
            choices.extend(f.popup_choices);
            log.extend(f.log);
        }
        assert_eq!(choices, vec![("replay.finished".to_owned(), 2)], "{log:?}");
        ms.set_replay(None);
        log.extend(ms.update_keys(Vec::new(), 1.0 / 60.0).log);
        log.extend(ms.update_keys(vec![(APT_START, 0)], 0.0).log);
        log.extend(run(&mut ms, 60));
        eprintln!("exit log: {log:?}");
        let mut levels = Vec::new();
        let draws = ms.stage.draw_levels(|key, movie| ms.movies.scene(key, movie), &mut levels).unwrap();
        eprintln!("levels drawn after the close: {:?} of {} draws", levels, draws.len());
        assert!(log.iter().any(|l| l.starts_with("close Replay2")), "{log:?}");
        assert!(!ms.replay_shown());
        assert!(ms.open(), "the pause menu opened again: {log:?}");
        let unanswered: Vec<&String> = ms.stage.unhandled().iter().filter(|c| c.contains("FEReplayMenu")).collect();
        assert!(unanswered.is_empty(), "FEReplayMenu natives left unanswered: {unanswered:?}");
        assert!(keys >= 1, "the timeline strip drew its key");
    }
}

#[cfg(test)]
mod signup_tests {
    use super::*;

    /// The teleporter prompt: the action overlay shows the title with its D-pad Up prompt, and hides again. Data-gated
    /// like the pause test (a set with `hud2/actionoverlay2`, version 11).
    #[test]
    fn the_teleporter_prompt_shows_and_hides() {
        let Some(root) = crate::fe::apt_imports::test_menu_set() else { return };
        let assets = crate::fe::apt_imports::main_checkout().join("assets");
        let read_set = |p: &str| std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"));
        let read = |p: &str| std::fs::read(assets.join(p)).map_err(|e| format!("{p}: {e}"));
        let movies = RetailMenuMovies::read_lean(&read_set, Some(&PAUSE_ROOTS)).unwrap();
        let mut ms = MenuStage::boot(movies, &read).unwrap();
        if !ms.has_signup() {
            eprintln!("skipped: no action overlay in this set");
            return;
        }
        let run = |ms: &mut MenuStage, frames: usize| {
            let mut log = Vec::new();
            for _ in 0..frames {
                log.append(&mut ms.update_keys(Vec::new(), 1.0 / 60.0).log);
            }
            assert!(!ms.failed, "{log:?}");
            log
        };
        run(&mut ms, 120);
        let mut levels = Vec::new();
        let idle = ms.stage.draw_levels(|key, movie| ms.movies.scene(key, movie), &mut levels).unwrap();
        eprintln!("idle levels {levels:?}: {:?}", idle.iter().map(|d| (d.texture.rsplit('/').next().unwrap_or("").to_owned(), d.multiply[3])).collect::<Vec<_>>());
        ms.draw();
        assert!(ms.prompts().is_empty(), "nothing on the stage while hidden: {:?}", ms.prompts());
        ms.set_signup(Some("Enter Stadium".into()));
        let log = run(&mut ms, 60);
        assert!(log.iter().any(|l| l.starts_with("signup state 2")), "{log:?}");
        ms.draw();
        assert!(ms.prompts().iter().any(|(n, _)| n.starts_with("DPad_Up")), "the D-pad Up prompt: {:?}", ms.prompts());
        ms.set_signup(None);
        run(&mut ms, 90);
        assert!(!ms.signup_active());
        ms.draw();
        assert!(ms.prompts().is_empty(), "hidden again: {:?}", ms.prompts());
    }
}

#[allow(dead_code)]
fn _assert_send() {
    fn send<T: Send>() {}
    send::<MenuStage>();
}
