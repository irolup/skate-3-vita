//! The retail list popup (`popup/sk8popup`, skate_vita P1): a description, a menu picker of
//! options and a Cancel / Select button bar between two frame lines. The game's side is the
//! `Popup` native (`GetDescription`, `GetNumOptions`, `GetOption`, `IsPopupCancelable`); the
//! host moves the picker with `UpdateInput` as the crossbar does and keeps the selection.
use crate::apt_scene::{self, Draw, Shapes};
use crate::apt_vm::{ObjectKind, Value, Vm};
use crate::crossbar::{APT_DOWN, APT_PREVIOUS, APT_UP};
use crate::menu_runtime::{MenuRuntime, Natives};

pub struct Host {
    pub description: String,
    pub options: Vec<String>,
    pub cancelable: bool,
}

impl Natives for Host {
    fn names(&self) -> Vec<String> {
        ["Popup", "Game", "ScreenManager", "Audio", "FELanguage"].iter().map(|s| (*s).to_owned()).collect()
    }
    fn constants(&self) -> Vec<(String, Value)> {
        [("AptUp", APT_UP), ("AptDown", APT_DOWN), ("AptPrevious", APT_PREVIOUS)].iter().map(|(k, v)| ((*k).to_owned(), Value::Number(*v))).collect()
    }
    fn call(&mut self, _: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Value, String> {
        let n = args.first().map_or(0, |v| v.number().max(0.0) as usize);
        Ok(match (native, method) {
            // `#` is the screen's "no text".
            ("Popup", "GetDescription") => Value::Text(if self.description.is_empty() { "#".into() } else { self.description.clone() }),
            ("Popup", "GetNumOptions") => Value::Number(self.options.len() as f64),
            ("Popup", "GetOption") => Value::Text(self.options.get(n).cloned().unwrap_or_default()),
            ("Popup", "IsPopupCancelable") => Value::Bool(self.cancelable),
            ("Game", "OnMenuGetItemEnabled") => Value::Bool(true),
            _ => Value::Undefined,
        })
    }
}

pub struct Popup {
    pub runtime: MenuRuntime<Host>,
    pub shapes: Shapes,
    selected: usize,
    shown: bool,
}

impl Popup {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
        let mut runtime = MenuRuntime::load(json, Host { description: String::new(), options: Vec::new(), cancelable: true })?;
        // The constructor's `createTextField('body', ...)` (not in this runtime): the layout reads
        // and writes the field's `_y`, so it gets a plain object to do that on (no description
        // text is ever shown).
        let root = runtime.bindings.movie.root;
        if let Value::Object(panel) = runtime.vm.get(root, "mcPanel") {
            let body = runtime.vm.object(ObjectKind::Plain);
            runtime.vm.set(body, "_y", Value::Number(0.0))?;
            runtime.vm.set(body, "maxscroll", Value::Number(0.0))?;
            runtime.vm.set(panel, "body", Value::Object(body))?;
        }
        apt_scene::size_buttons(&runtime.bindings.movie, &mut runtime.vm, &shapes)?;
        Ok(Self { runtime, shapes, selected: 0, shown: false })
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Shows the popup with these options (localization ids or text), the first selected.
    pub fn open(&mut self, options: Vec<String>, description: &str) -> Result<(), String> {
        let host = &mut self.runtime.bindings.natives;
        host.options = options;
        host.description = description.to_owned();
        self.selected = 0;
        self.shown = true;
        self.runtime.call_screen("SetState", vec![Value::Number(0.0)])?;
        // The screen manager's part: show it and play its intro.
        let root = self.runtime.bindings.movie.root;
        self.runtime.vm.set(root, "_visible", Value::Bool(true))?;
        self.runtime.call_screen("OnLoaded", vec![])?;
        // The picker writes its labels and its highlight only when the selection moves (they read "TBD TITLE"
        // until then): have it write them now.
        if let Value::Object(panel) = self.runtime.vm.get(root, "mcPanel") {
            if let Value::Object(picker) = self.runtime.vm.get(panel, "mMenuPicker") {
                self.runtime.vm.begin_update();
                self.runtime.vm.call_method(picker, "UpdateText", Vec::new(), &mut self.runtime.bindings)?;
                // And light the first option (the picker's `Update` selects it).
                self.runtime.vm.call_method(picker, "Update", Vec::new(), &mut self.runtime.bindings)?;
                self.runtime.drain()?;
            }
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.shown = false;
    }

    /// Moves the selection to `index` (one step at a time, as the D-pad would).
    pub fn select(&mut self, index: usize) -> Result<(), String> {
        let count = self.runtime.bindings.natives.options.len();
        if count == 0 {
            return Ok(());
        }
        let index = index % count;
        while self.selected != index {
            let down = (self.selected + 1) % count == index || index > self.selected && index - self.selected <= count / 2;
            self.selected = if down { (self.selected + 1) % count } else { (self.selected + count - 1) % count };
            let key = if down { APT_DOWN } else { APT_UP };
            self.runtime.call_screen("UpdateInput", vec![Value::Number(key), Value::Number(0.0)])?;
        }
        Ok(())
    }

    pub fn update(&mut self) -> Result<(), String> {
        self.runtime.update()
    }

    /// The controller glyphs of the button bar: (retail button name, stage x, y, width, height).
    pub fn glyphs(&self) -> Vec<(String, [f32; 4])> {
        apt_scene::text_fields(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes, "mButtonImage")
    }

    /// The popup over a darkened screen (the retail game dims what is behind it).
    pub fn draw(&self) -> Result<Vec<Draw>, String> {
        let mut draws = vec![apt_scene::dim(0.55)];
        draws.extend(apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes)?);
        draws.retain(|d| !(d.texture == apt_scene::WHITE && (d.multiply[1] - 254.0 / 255.0).abs() < 1e-3 && (d.multiply[2] - 234.0 / 255.0).abs() < 1e-3));
        Ok(draws)
    }
}
