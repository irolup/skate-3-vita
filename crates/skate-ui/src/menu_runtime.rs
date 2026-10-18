//! Runtime for the linked front-end screens (skate_vita, M9; `tools/link_ui_screen.py`):
//! the original screen movie (a crossbar, a popup) with its imported control bundles,
//! run like `hud_runtime::Runtime` runs the trick display, plus what the front end needs
//! and the trick display never did: `Object.registerClass` (symbols bound to their
//! classes), `attachMovie` / `removeMovieClip`, clip construction events and the class
//! constructors in Flash's order. The game's side of the screens (the data model the
//! scripts ask for, e.g. `coremenu.GetNumMenus`) is a `Natives` implementation.
use crate::apt_attach;
use crate::apt_movie::{CONSTRUCTOR, Movie};
use crate::apt_vm::{Host, ObjectKind, Value, Vm};
use std::collections::BTreeMap;

/// The game's native objects for a screen: which global objects exist and what their
/// methods answer.
pub trait Natives {
    /// Global native objects to define (`"coremenu"`, `"FELanguage"`, ...).
    fn names(&self) -> Vec<String>;
    /// Global constants to define before the scripts run (key codes, state numbers).
    fn constants(&self) -> Vec<(String, Value)> {
        Vec::new()
    }
    /// `native.method(args)`; unknown calls should return an error (or `Undefined`).
    fn call(&mut self, vm: &mut Vm, native: &str, method: &str, args: &[Value]) -> Result<Value, String>;
}

pub struct Bindings<N: Natives> {
    pub movie: Movie,
    pub exports: BTreeMap<String, i32>,
    pub natives: N,
    random: u32,
}

impl<N: Natives> Host for Bindings<N> {
    fn property_changed(&mut self, vm: &mut Vm, object: usize, key: &str) -> Result<(), String> {
        if key == "text" || key == "autoSize" {
            self.movie.text_changed(vm, object)?;
        }
        Ok(())
    }
    fn call(&mut self, vm: &mut Vm, object: usize, method: &str, args: Vec<Value>) -> Result<Value, String> {
        if self.movie.method(vm, object, method, &args)? {
            return Ok(Value::Undefined);
        }
        if self.movie.instances.contains_key(&object) {
            // skate_vita (P1): `setTextFormat` keeps the format's colour (the button menus'
            // `SetInfo`); the scene draws the field in it (`textColor`, as a script may also set).
            if method == "setTextFormat" {
                if let Some(Value::Object(format)) = args.first() {
                    let format = *format;
                    if let Value::Number(colour) = vm.get(format, "color") {
                        vm.set(object, "textColor", Value::Number(colour))?;
                    }
                    if let Value::Text(font) = vm.get(format, "font") {
                        vm.set(object, "textFont", Value::Text(font))?;
                    }
                    if let Value::Number(size) = vm.get(format, "size") {
                        vm.set(object, "textSize", Value::Number(size))?;
                    }
                    self.movie.text_changed(vm, object)?;
                }
                return Ok(Value::Undefined);
            }
            match method {
                "attachMovie" => return apt_attach::attach_movie(&mut self.movie, vm, &self.exports, object, &args),
                "removeMovieClip" => {
                    apt_attach::remove_movie_clip(&mut self.movie, vm, object)?;
                    return Ok(Value::Undefined);
                }
                _ => {}
            }
        }
        // Arrays are plain objects with numeric keys and a `length`.
        if matches!(vm.objects.get(object).map(|o| &o.kind), Some(ObjectKind::Plain)) {
            match method {
                "push" => {
                    let mut length = vm.get(object, "length").number();
                    if !length.is_finite() || length < 0.0 {
                        length = 0.0;
                    }
                    for value in args {
                        vm.set(object, (length as usize).to_string(), value)?;
                        length += 1.0;
                    }
                    vm.set(object, "length", Value::Number(length))?;
                    return Ok(Value::Number(length));
                }
                "pop" => {
                    let length = vm.get(object, "length").number();
                    if !length.is_finite() || length < 1.0 {
                        return Ok(Value::Undefined);
                    }
                    let key = (length as usize - 1).to_string();
                    let value = vm.get(object, &key);
                    vm.objects[object].fields.remove(&key);
                    vm.set(object, "length", Value::Number(length - 1.0))?;
                    return Ok(value);
                }
                _ => {}
            }
        }
        let native = match vm.objects.get(object).map(|o| &o.kind) {
            Some(ObjectKind::Native(name)) => name.clone(),
            _ => "global".to_owned(),
        };
        match (native.as_str(), method) {
            ("Object", "registerClass") => {
                let linkage = args.first().ok_or("registerClass: linkage")?.text();
                let Some(Value::Object(class)) = args.get(1) else {
                    return Err("registerClass: class".into());
                };
                let character = *self.exports.get(&linkage).ok_or_else(|| format!("registerClass: unknown linkage {linkage:?}"))?;
                self.movie.class_for.insert(character, *class);
                Ok(Value::Bool(true))
            }
            ("Math", "floor") => Ok(Value::Number(args.first().ok_or("Math.floor argument")?.number().floor())),
            // skate_vita (P1): the button menus' `GetWidth` and the replay editor's layout.
            ("Math", "ceil") => Ok(Value::Number(args.first().ok_or("Math.ceil argument")?.number().ceil())),
            ("Math", "random") => {
                // Presentation-only random source (as the trick display's).
                self.random ^= self.random << 13;
                self.random ^= self.random >> 17;
                self.random ^= self.random << 5;
                Ok(Value::Number(self.random as f64 / 4294967296.0))
            }
            ("global", "ASSetPropFlags") => Ok(Value::Undefined),
            (native, method) => self.natives.call(vm, native, method, &args),
        }
    }
}

pub struct MenuRuntime<N: Natives> {
    pub vm: Vm,
    pub bindings: Bindings<N>,
    /// `_root.screen`: the screen's controller object (the class the movie constructed).
    pub controller: Option<usize>,
}

impl<N: Natives> MenuRuntime<N> {
    pub fn load(json: &serde_json::Value, natives: N) -> Result<Self, String> {
        let mut vm = Vm::new();
        let mut names: Vec<String> = ["Object", "MovieClip", "Math"].iter().map(|s| (*s).to_owned()).collect();
        names.extend(natives.names());
        for name in names {
            let object = vm.object(ObjectKind::Native(name.clone()));
            let prototype = vm.object(ObjectKind::Plain);
            vm.set(object, "prototype", Value::Object(prototype))?;
            vm.set(vm.global, name, Value::Object(object))?;
        }
        vm.set(vm.global, "Screen_EdgeOffset", Value::Number(0.0))?;
        for (name, value) in natives.constants() {
            vm.set(vm.global, name, value)?;
        }
        let exports: BTreeMap<String, i32> = json
            .get("exports")
            .and_then(|e| e.as_object())
            .map(|o| o.iter().filter_map(|(k, v)| v.as_i64().map(|v| (k.clone(), v as i32))).collect())
            .unwrap_or_default();
        let mut movie = Movie::load(json)?;
        movie.max_instances = 16_384;
        let mut bindings = Bindings { movie, exports, natives, random: 0x9e37_79b9 };
        let initial: Vec<_> = bindings
            .movie
            .characters
            .values()
            .flat_map(|c| &c.frames)
            .flat_map(|f| &f.controls)
            .filter(|c| c.type_name == "do_init_action")
            .map(|c| c.actions_offset)
            .collect();
        for offset in initial {
            let code = bindings.movie.actions[&offset.to_string()].clone();
            vm.run(&code, &mut bindings)?;
        }
        vm.begin_update();
        bindings.movie.initialize(&mut vm)?;
        let mut runtime = Self { vm, bindings, controller: None };
        runtime.drain()?;
        if let Value::Object(controller) = runtime.vm.get(runtime.bindings.movie.root, "screen") {
            runtime.controller = Some(controller);
        }
        Ok(runtime)
    }

    /// Hides root-level clips by name (panels the host does not use).
    pub fn hide(&mut self, names: &[&str]) -> Result<(), String> {
        for name in names {
            if let Value::Object(clip) = self.vm.get(self.bindings.movie.root, name) {
                self.vm.set(clip, "_visible", Value::Bool(false))?;
            }
        }
        Ok(())
    }

    /// Runs the queued frame actions, clip construction events and class constructors.
    pub fn drain(&mut self) -> Result<(), String> {
        let mut calls = 0;
        while let Some((object, offset)) = self.bindings.movie.pending.pop_front() {
            calls += 1;
            if calls > 4096 {
                return Err("front-end frame action limit".into());
            }
            let Some(instance) = self.bindings.movie.instances.get(&object) else { continue };
            if offset == CONSTRUCTOR {
                let character = instance.character;
                let Some(&class) = self.bindings.movie.class_for.get(&character) else { continue };
                // `class.apply(clip)`: run the class function with the clip as `this`.
                self.vm.set(object, "__constructor__", Value::Object(class))?;
                let result = self.vm.call_method(object, "__constructor__", Vec::new(), &mut self.bindings);
                self.vm.objects[object].fields.remove("__constructor__");
                result?;
                continue;
            }
            let code = self
                .bindings
                .movie
                .actions
                .get(&offset.to_string())
                .ok_or_else(|| format!("missing action block {offset}"))?
                .clone();
            self.vm.run_on(object, &code, &mut self.bindings)?;
        }
        Ok(())
    }

    /// One movie frame: timelines advance, their actions run, unreachable objects go.
    pub fn update(&mut self) -> Result<(), String> {
        self.vm.begin_update();
        self.bindings.movie.advance(&mut self.vm)?;
        self.drain()?;
        self.vm.collect(self.bindings.movie.instances.keys().copied())?;
        Ok(())
    }

    /// Calls a method of the screen's controller, then runs what it queued.
    pub fn call_screen(&mut self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        let controller = self.controller.ok_or("the screen has no controller (_root.screen)")?;
        self.vm.begin_update();
        let result = self.vm.call_method(controller, method, args, &mut self.bindings)?;
        self.drain()?;
        Ok(result)
    }
}
