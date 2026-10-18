//! The retail trick analyzer (`hud2/trickanalyser`, skate_vita M9): the movie shows the
//! stick gesture of a trick (`g_ollie_kickflip`, `g_grind_fs_boardslide`, ... one clip per
//! gesture). Its script asks the game for `HUDComponents.TrickAnalyzer_GetTrickData()`,
//! an array `[gesture name, flag]` (`""` for none), when it receives `SetState(3)`;
//! `SetState(1)` shows it and `SetState(2)` hides it. The script mirrors the clip
//! (`_xscale = -100`, its trace says "GESTURE SET TO SWITCH") when the flag is true. The
//! clips show the regular-stance stick motion, and the simulation names a switch trick
//! by the feet (the kickflip motion in switch is a switch heelflip), so the flag is "riding
//! switch": the gesture then shows the motion the player made, as in the game.
//!
//! The reference has none of this. The game's own table from trick to gesture clip is in
//! the retail executable, so `gesture_for` rebuilds it from the names: a trick's
//! localization id (`ID_TRICK_FLIP_KICKFLIP`) and the clip names agree after dropping
//! underscores, for the flips (ollie or nollie clips) and the grinds (front-side clips,
//! used for the back-side grinds too); other tricks have no gesture clip.
use crate::apt_scene::{self, Draw, Shapes};
use crate::apt_vm::{ObjectKind, Value, Vm};
use crate::menu_runtime::{MenuRuntime, Natives};

pub struct Host {
    gesture: String,
    switch: bool,
}
impl Natives for Host {
    fn names(&self) -> Vec<String> {
        ["ScreenManager", "HUDComponents", "Tricks"].map(String::from).to_vec()
    }
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, _: &[Value]) -> Result<Value, String> {
        Ok(match (native, method) {
            ("HUDComponents", "TrickAnalyzer_GetTrickData") => {
                let array = vm.object(ObjectKind::Plain);
                vm.set(array, "0", Value::Text(self.gesture.clone()))?;
                vm.set(array, "1", Value::Bool(self.switch))?;
                vm.set(array, "length", Value::Number(2.0))?;
                Value::Object(array)
            }
            ("HUDComponents", "PlayerStance_IsRegular") => Value::Bool(self.switch),
            ("Tricks", "IsStanceTrickGesturesFlipped") => Value::Bool(false),
            _ => Value::Undefined,
        })
    }
}

fn normalized(s: &str) -> String {
    s.chars().filter(|c| *c != '_').flat_map(char::to_lowercase).collect()
}

/// The gesture clip (without the `g_`) for a trick's localization id, if there is one.
/// `exports` are the movie's linkage names; `nollie` picks the nollie clips (the stick
/// motion up, then down).
pub fn gesture_for(trick: &str, nollie: bool, exports: &[String]) -> Option<String> {
    let clips: std::collections::BTreeMap<String, &str> =
        exports.iter().filter_map(|e| e.strip_prefix("g_")).map(|c| (normalized(c), c)).collect();
    // Since b3c9679 a name can be composed (`ID_TRICK_FLIP_KICKFLIP 360`): the gesture
    // belongs to its first component.
    let t = trick.split_whitespace().next()?.strip_prefix("ID_TRICK_")?;
    let mut tries = Vec::new();
    if let Some(flip) = t.strip_prefix("FLIP_") {
        let n = normalized(flip);
        tries.push(if n.starts_with("nollie") { n } else { format!("{}{n}", if nollie { "nollie" } else { "ollie" }) });
    } else if let Some(grind) = t.strip_prefix("GRIND_") {
        if grind.starts_with("GRAB") {
            return None;
        }
        let n = normalized(grind);
        let fs = n.replacen("bs", "fs", 1);
        for a in [n, fs] {
            tries.push(format!("grindfs{}", a.trim_start_matches("fs")));
            tries.push(format!("grind{a}"));
            tries.push(format!("grind{a}grind"));
        }
    } else if t.starts_with("REVERT") {
        tries.push("revert".into());
    }
    tries.iter().find_map(|k| clips.get(k).map(|c| (*c).to_owned()))
}

pub struct TrickAnalyzer {
    pub runtime: MenuRuntime<Host>,
    pub shapes: Shapes,
    exports: Vec<String>,
    shown: String,
    shown_switch: bool,
}

impl TrickAnalyzer {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
        let exports = json["exports"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        let mut runtime = MenuRuntime::load(json, Host { gesture: String::new(), switch: false })?;
        runtime.call_screen("SetState", vec![Value::Number(1.0)])?;
        // `mTrickAnalyzer` is an authored placeholder (a flat blue square) where the game
        // draws its own content; the round backer behind it is what the game shows.
        runtime.hide(&["mTrickAnalyzer"])?;
        Ok(Self { runtime, shapes, exports, shown: String::new(), shown_switch: false })
    }

    /// Once per displayed frame. `trick` is the localization id being scored (`""` for
    /// none) and `switch` its stance; the last gesture stays until the next one (the caller
    /// decides how long).
    pub fn update(&mut self, trick: &str, nollie: bool, switch: bool) -> Result<(), String> {
        if let Some(gesture) = gesture_for(trick, nollie, &self.exports) {
            if gesture != self.shown || switch != self.shown_switch {
                let host = &mut self.runtime.bindings.natives;
                host.gesture = gesture.clone();
                host.switch = switch;
                self.shown = gesture;
                self.shown_switch = switch;
                self.runtime.call_screen("SetState", vec![Value::Number(3.0)])?;
            }
        }
        self.runtime.update()
    }

    /// The gesture clip on screen (without the `g_`), `""` for none.
    pub fn shown(&self) -> &str {
        &self.shown
    }

    /// Hides the gesture (no trick for a while).
    pub fn clear(&mut self) -> Result<(), String> {
        if !self.shown.is_empty() {
            self.shown.clear();
            self.runtime.bindings.natives.gesture.clear();
            self.runtime.call_screen("SetState", vec![Value::Number(3.0)])?;
        }
        Ok(())
    }

    pub fn draw(&self) -> Result<Vec<Draw>, String> {
        apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes)
    }
}
