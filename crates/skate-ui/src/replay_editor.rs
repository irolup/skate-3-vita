//! The retail replay editor (`replay2/replay_editor2`, skate_vita P1): the screen is a view.
//! In the retail game a native `FEReplayMenu` object owns the editing and answers the screen's
//! questions (playhead, markers, length, speed and shake settings); the host tells the screen
//! which mode to show with `SetState`. This is that host side, driven by the game's replay.
use crate::apt_scene::{self, Draw, Shapes};
use crate::apt_vm::{Value, Vm};
use crate::menu_runtime::{MenuRuntime, Natives};

/// `ReplayEditor.k*` state numbers of the screen's script.
pub mod state {
    pub const SCRUB: f64 = 0.0;
    pub const EFFECTS: f64 = 4.0;
    pub const REFRESH_EFFECTS: f64 = 5.0;
    pub const UPDATE_VALUES: f64 = 6.0;
    pub const FREE_CAMERA: f64 = 7.0;
    pub const UPDATE_ZOOM: f64 = 11.0;
    pub const MAIN_EDIT: f64 = 9.0;
    pub const START_PLAYBACK: f64 = 18.0;
    pub const STOP_PLAYBACK: f64 = 19.0;
    pub const MOVE_PLAYHEAD: f64 = 999.0;
    pub const MOVE_START: f64 = 1000.0;
    pub const MOVE_END: f64 = 1001.0;
    pub const SHOW_EDITOR: f64 = 1002.0;
    pub const HIDE_EDITOR: f64 = 1003.0;
    pub const OVER_CLIP_MARKER: f64 = 1006.0;
    pub const OVER_CLIP: f64 = 1007.0;
    pub const OVER_KEY_MARKER: f64 = 1009.0;
}

/// What the screen asks the game for.
#[derive(Clone, Debug)]
pub struct Model {
    /// Playhead, start and end markers as fractions of the timeline (0..1).
    pub cursor: f32,
    pub start: f32,
    pub end: f32,
    /// The trimmed clip's length as text and whether it is within the limit.
    pub length: String,
    pub valid: bool,
    pub zoom: String,
    pub camera_index: i32,
    pub speed: i32,
    pub shake: i32,
    /// The screen's intro has finished (`UpdateReplayEditorReady`).
    pub ready: bool,
    /// The instant replay is showing: the screen's `InstantDone` (run by its instant timeline) then
    /// leaves the mode alone instead of switching to the timeline editor.
    pub instant: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self { cursor: 0.0, start: 0.0, end: 1.0, length: "0:00".into(), valid: true, zoom: String::new(), camera_index: 2, speed: 0, shake: 0, ready: false, instant: false }
    }
}

pub struct Host {
    pub model: Model,
}

impl Natives for Host {
    fn names(&self) -> Vec<String> {
        ["FEReplayMenu", "Game", "ScreenManager", "FELanguage"].iter().map(|s| (*s).to_owned()).collect()
    }
    fn call(&mut self, _: &mut Vm, native: &str, method: &str, _: &[Value]) -> Result<Value, String> {
        let m = &mut self.model;
        Ok(match (native, method) {
            ("FEReplayMenu", "GetPlayheadPosition" | "GetCursorPosition") => Value::Number(f64::from(m.cursor)),
            ("FEReplayMenu", "GetStartHeadPosition") => Value::Number(f64::from(m.start)),
            ("FEReplayMenu", "GetEndHeadPosition") => Value::Number(f64::from(m.end)),
            ("FEReplayMenu", "GetEncodedLength") => Value::Text(m.length.clone()),
            ("FEReplayMenu", "IsEncodedLengthValid") => Value::Bool(m.valid),
            ("FEReplayMenu", "GetCameraZoomString") => Value::Text(m.zoom.clone()),
            ("FEReplayMenu", "GetCurrentCameraEditIndex" | "GetEditIndex") => Value::Number(f64::from(m.camera_index)),
            ("FEReplayMenu", "GetSimpleSpeedSetting") => Value::Number(f64::from(m.speed)),
            ("FEReplayMenu", "GetShakeCamType") => Value::Number(f64::from(m.shake)),
            // The editor shows its Take Photo item: a screenshot saved to the card.
            ("FEReplayMenu", "CanSaveMedia") => Value::Bool(true),
            ("FEReplayMenu", "CanDisplayShowHide") => Value::Bool(true),
            ("FEReplayMenu", "IsInstantReplayMode") => Value::Bool(m.instant),
            ("FEReplayMenu", "UpdateReplayEditorReady") => {
                m.ready = true;
                Value::Undefined
            }
            ("Game", "IsDemo") => Value::Bool(false),
            _ => Value::Undefined,
        })
    }
}

pub struct ReplayEditor {
    pub runtime: MenuRuntime<Host>,
    pub shapes: Shapes,
}

impl ReplayEditor {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
        let runtime = MenuRuntime::load(json, Host { model: Model::default() })?;
        let mut editor = Self { runtime, shapes };
        apt_scene::size_buttons(&editor.runtime.bindings.movie, &mut editor.runtime.vm, &editor.shapes)?;
        Ok(editor)
    }

    pub fn model(&mut self) -> &mut Model {
        &mut self.runtime.bindings.natives.model
    }

    /// The screen's mode (`ReplayEditor.SetState`).
    pub fn set_state(&mut self, state: f64) -> Result<(), String> {
        self.runtime.call_screen("SetState", vec![Value::Number(state)])?;
        self.restore_glyphs()
    }

    /// Plays the screen's intro (the root timeline fades the editor in; the constructor stops it).
    pub fn intro(&mut self) -> Result<(), String> {
        let root = self.runtime.bindings.movie.root;
        self.runtime.vm.begin_update();
        self.runtime.vm.call_method(root, "gotoAndPlay", vec![Value::Text("intro".into())], &mut self.runtime.bindings)?;
        self.runtime.drain()
    }

    /// A clip of the timeline (`mReplay.<name>`): the playhead, `mReplayIn` or `mReplayOut`.
    fn timeline_clip(&self, name: &str) -> Option<usize> {
        let vm = &self.runtime.vm;
        let Value::Object(class) = vm.get(vm.global, "replay_editor2") else { return None };
        let Value::Object(screen) = vm.get(class, "mScreen") else { return None };
        let Value::Object(replay) = vm.get(screen, "mReplay") else { return None };
        match vm.get(replay, name) {
            Value::Object(clip) => Some(clip),
            _ => None,
        }
    }

    /// The stage x of the playhead, the In marker and the Out marker (for calibrating the host's timeline band).
    pub fn timeline_positions(&self) -> [Option<[f32; 2]>; 3] {
        ["mPlayhead", "mReplayIn", "mReplayOut"].map(|n| {
            let id = self.timeline_clip(n)?;
            apt_scene::world_position(&self.runtime.bindings.movie, &self.runtime.vm, id)
        })
    }

    /// Puts the In and Out point markers at the clip's `start` and `end` (fractions of the timeline)
    /// and shows or hides them (`None` leaves that to the screen). The screen's own placement
    /// (`GetCursorOffset`) follows the playhead whatever it is asked for, so the host sets them.
    pub fn place_range_markers(&mut self, start: f32, end: f32, shown: Option<bool>) -> Result<(), String> {
        let Some(controller) = self.runtime.controller else { return Ok(()) };
        let base = self.runtime.vm.get(controller, "mFrameCursorOffset").number();
        if !base.is_finite() {
            return Ok(());
        }
        for (name, fraction) in [("mReplayIn", start), ("mReplayOut", end)] {
            if let Some(id) = self.timeline_clip(name) {
                self.runtime.vm.set(id, "_x", Value::Number(base + f64::from(fraction) * 817.0))?;
                if let Some(shown) = shown {
                    self.runtime.vm.set(id, "_visible", Value::Bool(shown))?;
                }
            }
        }
        Ok(())
    }

    /// One displayed frame.
    pub fn update(&mut self) -> Result<(), String> {
        self.runtime.update()?;
        self.restore_glyphs()
    }

    /// The icon clips' button names (what the constructor gave them with `SetButtonType`). A
    /// mode change re-places the timeline's clips, and the new copies have none, so the host's
    /// glyphs would vanish: they are put back on every frame.
    fn restore_glyphs(&mut self) -> Result<(), String> {
        const ICONS: [(&str, &str, &str); 17] = [
            ("mcPlay", "mButtonItem", "R_Bumper_hud"),
            ("mcRewind", "mButtonItem", "L_Trigger_hud"),
            ("mcForward", "mButtonItem", "R_Trigger_hud"),
            ("mcStart", "mButtonItem", "DPad_Left_hud"),
            ("mcEnd", "mButtonItem", "DPad_Right_hud"),
            ("mcDropMarker", "mButtonItem", "DPad_Down_hud"),
            ("mcRemoveMarker", "mButtonItem", "DPad_Up_hud"),
            ("mcPrevMarker", "mButtonItem", "DPad_Left_hud"),
            ("mcNextMarker", "mButtonItem", "DPad_Right_hud"),
            ("mcLBMove", "mButtonItem1", "L_Bumper_hud"),
            ("mcLBMove", "mButtonItem2", "Stick_Left_hud"),
            ("mcRBMove", "mButtonItem1", "L_Bumper_hud"),
            ("mcRBMove", "mButtonItem2", "Stick_Right_hud"),
            ("mcCamRotate", "mButtonItem", "Stick_Right_hud"),
            ("mcCamLR", "mButtonItem", "Stick_Left_hud"),
            ("mcCamUpDown", "mButtonItem", "DPad_UpDown_hud"),
            ("mcCamZoom", "mButtonItem", "DPad_Arrows_hud"),
        ];
        for (clip, item, name) in ICONS {
            let Some(clip) = self.timeline_clip(clip) else { continue };
            if let Value::Object(item) = self.runtime.vm.get(clip, item) {
                if !matches!(self.runtime.vm.get(item, "mButtonImage"), Value::Text(_)) {
                    self.runtime.vm.set(item, "mButtonImage", Value::Text(name.into()))?;
                }
            }
        }
        Ok(())
    }

    /// The controller glyphs the screen leaves to the host: (retail button name, x, y, width, height)
    /// in the 1280 x 720 stage.
    pub fn glyphs(&self) -> Vec<(String, [f32; 4])> {
        apt_scene::text_fields(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes, "mButtonImage")
    }

    /// The screen's drawing without the white squares that stand for the controller glyphs
    /// (the host draws those: see `glyphs`).
    pub fn draw(&self) -> Result<Vec<Draw>, String> {
        let mut draws = apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes)?;
        draws.retain(|d| !(d.texture == apt_scene::WHITE && (d.multiply[1] - 254.0 / 255.0).abs() < 1e-3 && (d.multiply[2] - 234.0 / 255.0).abs() < 1e-3));
        Ok(draws)
    }
}

/// The retail replay help pages (`replay2/replay_help`): which page set it shows follows the
/// editor mode (`FEReplayMenu.GetHelpMode`); the A button turns the page (`SetState(0)`).
pub struct HelpHost {
    pub mode: i32,
}

impl Natives for HelpHost {
    fn names(&self) -> Vec<String> {
        ["FEReplayMenu", "ScreenManager", "Audio"].iter().map(|s| (*s).to_owned()).collect()
    }
    fn call(&mut self, _: &mut Vm, native: &str, method: &str, _: &[Value]) -> Result<Value, String> {
        Ok(match (native, method) {
            ("FEReplayMenu", "GetHelpMode") => Value::Number(f64::from(self.mode)),
            _ => Value::Undefined,
        })
    }
}

pub struct ReplayHelp {
    pub runtime: MenuRuntime<HelpHost>,
    pub shapes: Shapes,
}

impl ReplayHelp {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
        let runtime = MenuRuntime::load(json, HelpHost { mode: 0 })?;
        Ok(Self { runtime, shapes })
    }

    /// Shows the first page of a mode's help.
    pub fn open(&mut self, mode: i32) -> Result<(), String> {
        self.runtime.bindings.natives.mode = mode;
        let controller = self.runtime.controller.ok_or("the help screen has no controller")?;
        self.runtime.vm.set(controller, "mMode", Value::Number(f64::from(mode)))?;
        self.runtime.vm.set(controller, "mPage", Value::Number(0.0))?;
        self.runtime.call_screen("UpdateHelpTitle", vec![]).map(|_| ())
    }

    /// The A button: the next page (the first again after the last).
    pub fn next_page(&mut self) -> Result<(), String> {
        self.runtime.call_screen("SetState", vec![Value::Number(0.0)]).map(|_| ())
    }

    pub fn update(&mut self) -> Result<(), String> {
        self.runtime.update()
    }

    pub fn glyphs(&self) -> Vec<(String, [f32; 4])> {
        apt_scene::text_fields(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes, "mButtonImage")
    }

    pub fn draw(&self) -> Result<Vec<Draw>, String> {
        let mut draws = apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes)?;
        draws.retain(|d| !(d.texture == apt_scene::WHITE && (d.multiply[1] - 254.0 / 255.0).abs() < 1e-3 && (d.multiply[2] - 234.0 / 255.0).abs() < 1e-3));
        Ok(draws)
    }
}
