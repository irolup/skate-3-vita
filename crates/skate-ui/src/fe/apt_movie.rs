//! Original APT movie hierarchy and timeline control, independent of Bevy.
use crate::fe::{
    apt_display::{clip_event, ClipAction, Control, DisplayList, Placement},
    apt_vm::{Instruction, ObjectKind, Value, Vm},
};
use serde::Deserialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

#[derive(Clone, Deserialize)]
pub struct Frame {
    pub controls: Vec<Control>,
}
#[derive(Clone, Deserialize)]
pub struct Character {
    pub id: i32,
    pub type_name: String,
    #[serde(default)]
    pub frames: Vec<Frame>,
    pub text: Option<serde_json::Value>,
    pub bounds: Option<[f32; 4]>,
}
#[derive(Clone)]
pub struct Instance {
    pub character: i32,
    pub frame: usize,
    pub playing: bool,
    pub children: BTreeMap<i32, usize>,
    pub placement: Option<Placement>,
    /// skate_vita (performance): the depth list of the frame last placed (clips with more than one frame), so the
    /// next frame applies only its own controls to it (the same list as `display_list`: every control applies in
    /// frame order onto the previous frame's list).
    cursor: Option<(usize, DisplayList)>,
    /// skate_vita (performance): the movie's advance count when the clip was made (`Movie::ticks`): `advance` ticks the
    /// clips that existed before it, without a list of every id.
    born: u64,
    /// skate_vita (performance): the clip's last frame placement still stands (children = that frame's placed clips
    /// plus attached ones, each with that frame's placement), with the unresolved set it used. Whatever changes the
    /// children (attachMovie, createTextField, removeMovieClip) clears it; `seek` sets it.
    settled: Option<Arc<std::collections::BTreeSet<i32>>>,
}
/// One queued action stream: a frame action (`event` 0) or a clip-event handler (doc 31,
/// Milestone 3d). It runs with `this` = `object`, the clip that owns the frame or the
/// placement (retail: the action queue entry's target, sub_82E78168 / sub_82E78240).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub object: usize,
    pub offset: u32,
    pub event: u32,
}
/// skate_vita (performance): a movie's clips by id (a VM object id, dense), in a vector indexed by id: the timeline
/// looks a clip up several times per placed child every frame, and upstream's `BTreeMap` walk cost the Vita more than
/// the rest of the frame. Same interface as the map (iteration in ascending id order).
#[derive(Clone, Default)]
pub struct Instances {
    slots: Vec<Option<Box<(usize, Instance)>>>,
    count: usize,
}

impl Instances {
    pub fn get(&self, id: &usize) -> Option<&Instance> {
        self.slots.get(*id)?.as_ref().map(|e| &e.1)
    }
    pub fn get_mut(&mut self, id: &usize) -> Option<&mut Instance> {
        self.slots.get_mut(*id)?.as_mut().map(|e| &mut e.1)
    }
    pub fn contains_key(&self, id: &usize) -> bool {
        self.slots.get(*id).is_some_and(Option::is_some)
    }
    pub fn insert(&mut self, id: usize, instance: Instance) -> Option<Instance> {
        if self.slots.len() <= id {
            self.slots.resize_with(id + 1, || None);
        }
        let old = self.slots[id].replace(Box::new((id, instance))).map(|e| e.1);
        if old.is_none() {
            self.count += 1;
        }
        old
    }
    pub fn remove(&mut self, id: &usize) -> Option<Instance> {
        let old = self.slots.get_mut(*id)?.take().map(|e| e.1);
        if old.is_some() {
            self.count -= 1;
        }
        old
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn clear(&mut self) {
        self.slots.clear();
        self.count = 0;
    }
    pub fn iter(&self) -> impl Iterator<Item = (&usize, &Instance)> + '_ {
        self.slots.iter().flatten().map(|e| (&e.0, &e.1))
    }
    pub fn keys(&self) -> impl Iterator<Item = &usize> + '_ {
        self.slots.iter().flatten().map(|e| &e.0)
    }
    pub fn values(&self) -> impl Iterator<Item = &Instance> + '_ {
        self.slots.iter().flatten().map(|e| &e.1)
    }
}

impl std::ops::Index<&usize> for Instances {
    type Output = Instance;
    fn index(&self, id: &usize) -> &Instance {
        self.get(id).expect("APT absent movie instance")
    }
}

impl<'a> IntoIterator for &'a Instances {
    type Item = (&'a usize, &'a Instance);
    type IntoIter = Box<dyn Iterator<Item = (&'a usize, &'a Instance)> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// `Pending::event` of a registerClass constructor call (not a clip event bit; the stage runs
/// the class function on the clip instead of an action stream).
pub const CLASS_CONSTRUCTOR: u32 = 1 << 31;
#[derive(Clone)]
pub struct Movie {
    /// skate_vita (memory): `characters`, `actions` and the display states are shared by the clones of a movie (every
    /// level a movie plays in is a clone of the set's copy); the few writes (linking, createTextField,
    /// setTextFormat) copy the map first.
    pub characters: Arc<BTreeMap<i32, Character>>,
    pub instances: Instances,
    /// skate_vita (performance): how many times the movie advanced (see `Instance::born`).
    ticks: u64,
    /// skate_vita (performance): the linked characters no library resolved (`linkage` values without a character),
    /// the ones a placement leaves absent; None = to compute again (characters or linkage changed).
    unresolved: Option<Arc<std::collections::BTreeSet<i32>>>,
    pub actions: Arc<BTreeMap<String, Vec<Instruction>>>,
    /// The retail action queue (ring buffer at global+24 -> +12): frame actions and most
    /// clip events go to the back (sub_82E78168), enterFrame to the front (sub_82E78240).
    pub pending: VecDeque<Pending>,
    /// Initialize / construct handlers. Retail runs them synchronously inside the placement
    /// (sub_82E7AF18 -> sub_82E5B158 -> sub_82E6A208), before anything already queued.
    /// NOT RETAIL (timing): when a running action places the clip (gotoAndStop), ours run
    /// right after that action instead of inside it; still before every other queued action.
    pub immediate: VecDeque<Pending>,
    pub root: usize,
    pub text_assets: crate::fe::apt_text::TextAssets,
    /// Shape characters copied in by `link_imports`: local id -> (library key, id there). Their GEO
    /// units and bitmaps stay in the library movie (doc 31, Milestone 4).
    pub shape_origins: BTreeMap<i32, (String, i32)>,
    /// Root timeline rate from the movie header (`milliseconds_per_frame` of character 0),
    /// when the export carries it; every movie keeps its own rate (core_menu 16 ms, dimmer 83 ms).
    pub milliseconds_per_frame: Option<f32>,
    /// Linkage names (import and export records) -> character id; `Object.registerClass` binds
    /// a class to a clip by this name.
    pub linkage: BTreeMap<String, i32>,
    /// The linkage name of every library character copied in (character -> name), also where the movie's own
    /// name wins in `linkage`: registerClass binds the library's symbol, so every copy gets the class (the trick
    /// guide imports "Button Item" itself and again through panel > button_menu; its button help had no class).
    pub class_links: BTreeMap<i32, String>,
    /// Library movie keys this movie imports from, in record order.
    pub import_files: Vec<String>,
    /// Classes bound by registerClass: character -> class function object. A new clip of that
    /// character gets the class prototype and a constructor call (`CLASS_CONSTRUCTOR`).
    pub classes: BTreeMap<i32, usize>,
    /// Clips whose position a script set: the timeline no longer moves them (Flash rule; NOT RETAIL YET, the
    /// retail flag is not traced). Only hosts that call `script_moved` use it (the front-end stage).
    pub script_moved: std::collections::BTreeSet<usize>,
    /// Clips made by attachMovie: their parent's timeline keeps them (they sit above every timeline depth).
    pub attached: std::collections::BTreeSet<usize>,
    /// Script-set scale / rotation (`_xscale`, `_yscale` in percent, `_rotation` in degrees; also from `_width` /
    /// `_height` writes): replaces the placement's linear part when drawing (standard AS2; NOT RETAIL YET: the
    /// retail property handlers are not decoded).
    pub scaled: BTreeMap<usize, [f32; 3]>,
    states: Arc<BTreeMap<i32, DisplayStates>>,
}

/// attachMovie depth d is child key d + SCRIPT_DEPTH, above every timeline depth (Flash: timeline depths start at
/// -16384, script depths at 0). NOT RETAIL YET: retail's depth split is not traced.
pub const SCRIPT_DEPTH: i32 = 16384;
/// Character ids of text fields made by createTextField (each has its own descriptor; below every disc id).
pub const SCRIPT_TEXT_BASE: i32 = -2_000_000;

/// `_xscale` / `_yscale` / `_rotation` of a placement matrix (AS2 reads them from the clip's matrix).
fn seed_scale(vm: &mut Vm, id: usize, m: &[f32; 6]) -> Result<(), String> {
    vm.set(id, "_xscale", Value::Number((100.0 * m[0].hypot(m[1])) as f64))?;
    vm.set(id, "_yscale", Value::Number((100.0 * m[2].hypot(m[3])) as f64))?;
    vm.set(id, "_rotation", Value::Number(m[1].atan2(m[0]).to_degrees() as f64))?;
    Ok(())
}

/// A clip's local matrix: the placement's (or identity), its linear part replaced by a script-set scale /
/// rotation, translated by `_x` / `_y`.
pub fn clip_matrix(movie: &Movie, vm: &Vm, id: usize) -> [f32; 6] {
    let mut m = movie
        .instances
        .get(&id)
        .and_then(|i| i.placement.as_ref())
        .map_or([1., 0., 0., 1., 0., 0.], |p| p.matrix);
    if let Some([sx, sy, rotation]) = movie.scaled.get(&id) {
        let (sin, cos) = rotation.to_radians().sin_cos();
        m[0] = cos * sx / 100.0;
        m[1] = sin * sx / 100.0;
        m[2] = -sin * sy / 100.0;
        m[3] = cos * sy / 100.0;
    }
    m[4] = vm.get(id, "_x").number() as f32;
    m[5] = vm.get(id, "_y").number() as f32;
    m
}
/// skate_vita (memory): a character's depth list at each frame, kept as one snapshot every [`STATE_STRIDE`] frames
/// (upstream kept every frame's list: 19 MB for the pause menu's movies). A frame between two snapshots is the one
/// before it with the next frames' controls applied again, which gives the same list: `DisplayList::apply` depends
/// only on the list and the control, and every control was applied once at load.
#[derive(Clone, Debug, Default)]
struct DisplayStates {
    snapshots: Vec<DisplayList>,
}
const STATE_STRIDE: usize = 8;

fn display_states(c: &Character) -> Result<DisplayStates, String> {
    let mut list = DisplayList::default();
    let mut snapshots = Vec::new();
    for (i, frame) in c.frames.iter().enumerate() {
        for control in &frame.controls {
            list.apply(control)?;
        }
        if i % STATE_STRIDE == 0 {
            snapshots.push(list.clone());
        }
    }
    Ok(DisplayStates { snapshots })
}

/// The depth list of `c` at `frame` (upstream's `states[c][frame]`).
fn display_list<'a>(c: &Character, states: &'a DisplayStates, frame: usize) -> Result<std::borrow::Cow<'a, DisplayList>, String> {
    let base = frame / STATE_STRIDE;
    let snapshot = states.snapshots.get(base).ok_or("APT frame without a display state")?;
    if frame % STATE_STRIDE == 0 {
        return Ok(std::borrow::Cow::Borrowed(snapshot));
    }
    let mut list = snapshot.clone();
    for f in c.frames.get(base * STATE_STRIDE + 1..=frame).ok_or("APT frame without a display state")? {
        for control in &f.controls {
            list.apply(control)?;
        }
    }
    Ok(std::borrow::Cow::Owned(list))
}
impl Movie {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        Self::load_text(json, crate::fe::apt_text::TextAssets::load(json)?)
    }

    /// skate_vita (memory): [`Self::load`] of a set's movie whose shared fields are not merged in (`shared`: the
    /// set's language and fonts, built once).
    pub fn load_shared(json: &serde_json::Value, shared: &crate::fe::apt_text::SharedText) -> Result<Self, String> {
        Self::load_text(json, crate::fe::apt_text::TextAssets::with_shared(json, shared)?)
    }

    fn load_text(json: &serde_json::Value, text_assets: crate::fe::apt_text::TextAssets) -> Result<Self, String> {
        let characters: Vec<Character> =
            serde_json::from_value(json["characters"].clone()).map_err(|e| e.to_string())?;
        let mut states = BTreeMap::new();
        for c in &characters {
            states.insert(c.id, display_states(c)?);
        }
        Ok(Self {
            characters: Arc::new(characters.into_iter().map(|c| (c.id, c)).collect()),
            instances: Instances::default(),
            ticks: 0,
            unresolved: None,
            actions: Arc::new(serde_json::from_value(json["actions"].clone()).map_err(|e| e.to_string())?),
            pending: VecDeque::new(),
            immediate: VecDeque::new(),
            root: usize::MAX,
            text_assets,
            shape_origins: BTreeMap::new(),
            milliseconds_per_frame: json["characters"]
                .as_array()
                .and_then(|c| c.iter().find(|c| c["id"] == 0))
                .and_then(|c| c["movie"]["milliseconds_per_frame"].as_f64())
                .filter(|ms| *ms > 0.0)
                .map(|ms| ms as f32),
            linkage: ["imports", "exports"]
                .iter()
                .flat_map(|k| json[*k].as_array().into_iter().flatten())
                .filter_map(|r| Some((r["name"].as_str()?.to_string(), r["character_id"].as_i64()? as i32)))
                .collect(),
            class_links: BTreeMap::new(),
            import_files: {
                let mut files: Vec<String> = Vec::new();
                for r in json["imports"].as_array().into_iter().flatten() {
                    if let Some(f) = r["file"].as_str() {
                        let key = crate::fe::apt_imports::library_key(f);
                        if !files.contains(&key) {
                            files.push(key);
                        }
                    }
                }
                files
            },
            classes: BTreeMap::new(),
            script_moved: Default::default(),
            attached: Default::default(),
            scaled: Default::default(),
            states: Arc::new(states),
        })
    }
    pub fn initialize(&mut self, vm: &mut Vm) -> Result<(), String> {
        self.initialize_into(vm, None)
    }
    /// Builds the root timeline. With `into`, the root reuses that script object (a clip a
    /// MovieClipLoader loads this movie into keeps its identity, name and parent, so script
    /// references to it, like ScreenManager.mMovieClipList, still reach the loaded screen).
    pub fn initialize_into(&mut self, vm: &mut Vm, into: Option<usize>) -> Result<(), String> {
        self.root = self.create_into(vm, 0, None, 0, &[], into)?;
        vm.set(self.root, "_root", Value::Object(self.root))?;
        // Every clip sees the same authored root, including unnamed children.
        for id in self.instances.keys() {
            vm.set(*id, "_root", Value::Object(self.root))?;
        }
        Ok(())
    }
    fn create(
        &mut self,
        vm: &mut Vm,
        character: i32,
        parent: Option<usize>,
        depth: usize,
        events: &[ClipAction],
    ) -> Result<usize, String> {
        self.create_into(vm, character, parent, depth, events, None)
    }
    fn create_into(
        &mut self,
        vm: &mut Vm,
        character: i32,
        parent: Option<usize>,
        depth: usize,
        events: &[ClipAction],
        into: Option<usize>,
    ) -> Result<usize, String> {
        // Ours (a guard, not a retail rule): the Challenge Map pre-allocates 11 list rows of ~112 clips, ~2838 on
        // load (b260), so 2048 was too low.
        if depth > 32 || self.instances.len() > 16384 {
            return Err(format!("APT movie hierarchy limit (depth {depth}, {} instances, character {character})", self.instances.len()));
        }
        // skate_vita (performance): read through a handle on the shared map (upstream copied the character, every
        // frame's controls, for each clip it created: the bulk of opening a screen).
        let characters = Arc::clone(&self.characters);
        let c = characters.get(&character).ok_or("APT unknown character")?;
        let kind = ObjectKind::Native(format!("movie:{character}"));
        let id = match into {
            Some(id) => {
                vm.objects.get_mut(id).ok_or("Invalid APT load target")?.kind = kind;
                id
            }
            None => vm.object(kind),
        };
        if self.root != usize::MAX {
            vm.set(id, "_root", Value::Object(self.root))?;
        }
        if let Some(parent) = parent {
            vm.set(id, "_parent", Value::Object(parent))?;
        }
        vm.set(id, "_x", Value::Number(0.0))?;
        vm.set(id, "_y", Value::Number(0.0))?;
        vm.set(id, "_visible", Value::Bool(true))?;
        vm.set(id, "_alpha", Value::Number(100.0))?;
        if let Some(text) = &c.text {
            vm.set(
                id,
                "text",
                Value::Text(text["initial_text"].as_str().unwrap_or("").into()),
            )?;
        }
        self.instances.insert(
            id,
            Instance {
                character,
                frame: 0,
                playing: !c.frames.is_empty(),
                children: BTreeMap::new(),
                placement: None,
                cursor: None,
                born: self.ticks,
                settled: None,
            },
        );
        self.text_changed(vm, id)?;
        if c.text.is_none() {
            if let Some(b) = self.character_bounds(character, 0) {
                vm.set(id, "_width", Value::Number((b[2] - b[0]) as f64))?;
                vm.set(id, "_height", Value::Number((b[3] - b[1]) as f64))?;
            }
        }
        // Retail sub_82E7AF18 (sprites only): initialize, then construct, both before the
        // new clip's own frame runs.
        let sprite = c.type_name == "sprite";
        if sprite {
            self.queue_events(id, events, clip_event::INITIALIZE, true);
            self.queue_events(id, events, clip_event::CONSTRUCT, true);
        }
        // registerClass: the clip takes the class prototype now. Retail sub_82E5A9E0 (after
        // sub_82E7AF18, from both the timeline place 82E7B090 and attachMovie 82E565A0; research
        // b452) sets the prototype, runs the first tick 82E5B640 (which places the children, so
        // their construct handlers run first), then the constructor (82E6A208). So the
        // constructor is queued after the children are placed; it still runs before the clip's
        // own frame actions, which wait on `pending`. A child's construct value (button_item2's
        // mButtonRender `_test = "Unknown"`) no longer overwrites what the parent's constructor
        // set (`SetButtonType(mButtonImage)`, the teleport prompt's DPad_Up).
        let class = self.classes.get(&character).copied();
        if let Some(class) = class {
            if let Value::Object(proto) = vm.get(class, "prototype") {
                vm.objects[id].prototype = Some(proto);
            }
        }
        if !c.frames.is_empty() {
            self.seek(vm, id, 0, depth + 1)?;
        }
        if class.is_some() {
            self.immediate.push_back(Pending {
                object: id,
                offset: 0,
                event: CLASS_CONSTRUCTOR,
            });
        }
        // Retail sub_82E5B640: the first tick of a new clip dispatches load (not
        // enterFrame), after that frame's actions were queued.
        // NOT RETAIL YET (order): retail ticks children after the parent queued its own
        // frame actions; our seek places (and loads) children first, as for frame actions.
        if sprite {
            self.queue_events(id, events, clip_event::LOAD, false);
        }
        Ok(id)
    }
    /// A script wrote `key` (`_xscale`, `_yscale`, `_rotation`, `_width`, `_height`) on a clip: record its
    /// transform. `_width` / `_height` of a non-text clip set the scale through its natural bounds (text fields
    /// keep their own `_width` meaning, the field box).
    pub fn transform_changed(&mut self, vm: &mut Vm, id: usize, key: &str) -> Result<(), String> {
        let Some(instance) = self.instances.get(&id) else { return Ok(()) };
        let character = instance.character;
        let is_text = self.characters.get(&character).is_some_and(|c| c.text.is_some());
        let key = key.to_ascii_lowercase();
        if is_text && (key == "_width" || key == "_height") {
            return Ok(());
        }
        let number = |vm: &Vm, k: &str| vm.get(id, k).number() as f32;
        let mut t = [number(vm, "_xscale"), number(vm, "_yscale"), number(vm, "_rotation")];
        if key == "_width" || key == "_height" {
            let Some(b) = self.character_bounds(character, 0) else { return Ok(()) };
            let (natural, axis) = if key == "_width" { (b[2] - b[0], 0) } else { (b[3] - b[1], 1) };
            if natural > 0.0 {
                t[axis] = 100.0 * number(vm, &key) / natural;
                vm.set(id, if axis == 0 { "_xscale" } else { "_yscale" }, Value::Number(t[axis] as f64))?;
            }
        } else if let Some(b) = self.character_bounds(character, 0) {
            vm.set(id, "_width", Value::Number(((b[2] - b[0]) * t[0] / 100.0) as f64))?;
            vm.set(id, "_height", Value::Number(((b[3] - b[1]) * t[1] / 100.0) as f64))?;
        }
        if t.iter().all(|v| v.is_finite()) {
            self.scaled.insert(id, t);
        }
        Ok(())
    }

    /// A character's bounds in its own space: its own (shapes, text) or, for a sprite, the union of its frame-0
    /// children through their matrices, leaving out clip-depth children like retail's bounds walk sub_82E59C28.
    /// New clips get `_width` / `_height` from it. NOT RETAIL YET: static frame-0 bounds, not re-measured when the
    /// clip's content changes.
    pub fn character_bounds(&self, character: i32, depth: usize) -> Option<[f32; 4]> {
        if depth > 16 {
            return None;
        }
        let c = self.characters.get(&character)?;
        if let Some(b) = c.bounds {
            return Some(b);
        }
        let list = self.states.get(&character)?.snapshots.first()?;
        let mut out: Option<[f32; 4]> = None;
        for p in list.depths.values().filter(|p| p.clip_depth < 0) {
            let Some(b) = self.character_bounds(p.character, depth + 1) else { continue };
            let m = p.matrix;
            for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])] {
                let (tx, ty) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
                out = Some(match out {
                    Some(o) => [o[0].min(tx), o[1].min(ty), o[2].max(tx), o[3].max(ty)],
                    None => [tx, ty, tx, ty],
                });
            }
        }
        out.filter(|o| o.iter().all(|v| v.is_finite()))
    }
    /// TextField.setTextFormat / setNewTextFormat: the format's colour becomes the field's text colour; on a field
    /// made by createTextField its font, size and alignment replace the field's own (authored fields keep theirs).
    /// NOT RETAIL YET: standard AS2 semantics, whole field.
    pub fn set_text_format(&mut self, vm: &mut Vm, id: usize, format: usize) -> Result<(), String> {
        let color = vm.get(format, "color");
        if !matches!(color, Value::Undefined) {
            vm.set(id, "textColor", color)?;
        }
        let Some(character) = self.instances.get(&id).map(|i| i.character).filter(|c| *c <= SCRIPT_TEXT_BASE) else {
            return Ok(());
        };
        let font = match vm.get(format, "font") {
            Value::Undefined => None,
            name => self.text_assets.font_named(&name.text()),
        };
        let size = vm.get(format, "size");
        let align = vm.get(format, "align").text();
        if let Some(text) = Arc::make_mut(&mut self.characters).get_mut(&character).and_then(|c| c.text.as_mut()) {
            if let Some(font) = font {
                text["font_id"] = font.into();
            }
            if let Value::Number(size) = size {
                text["font_height"] = size.into();
            }
            // 82CA23B4: 1 right, 2 / 3 centred, else left.
            match align.as_str() {
                "center" => text["alignment"] = 2.into(),
                "right" => text["alignment"] = 1.into(),
                "left" => text["alignment"] = 0.into(),
                _ => {}
            }
        }
        self.text_changed(vm, id)
    }

    pub fn text_changed(&self, vm: &mut Vm, id: usize) -> Result<(), String> {
        let Some(instance) = self.instances.get(&id) else {
            return Ok(());
        };
        let character = &self.characters[&instance.character];
        let Some(text) = &character.text else {
            return Ok(());
        };
        let value = self.text_assets.localize(&vm.get(id, "text").text());
        vm.set(id, "_displayText", Value::Text(value.clone()))?;
        let font = self
            .text_assets
            .fonts
            .get(&(text["font_id"].as_i64().ok_or("Invalid text font id")? as i32))
            .ok_or("Missing original text font")?;
        let height = text["font_height"].as_f64().ok_or("Invalid text size")? as f32;
        let width = font.width(&value, height);
        vm.set(id, "textWidth", Value::Number(width as f64))?;
        // NOT RETAIL YET: one line of the font size (button_menu centres its labels with textHeight); multi-line
        // measurement is not decoded.
        vm.set(id, "textHeight", Value::Number(height as f64))?;
        let bounds = character.bounds.ok_or("Missing text bounds")?;
        // `maxscroll`: the field's line count after the word wrap the renderer applies (apt_scene). sk8popup sizes its
        // panel with body.maxscroll * bodyLineHeight, i.e. as the line count. NOT RETAIL YET: APT's maxscroll is not
        // decoded (Flash's would be 1 for text that fits); the line count is what the popup script needs.
        let lines = if text["word_wrap"].as_bool() == Some(true) || vm.get(id, "wordWrap") == Value::Bool(true) {
            font.wrap(&value, height, bounds[2] - bounds[0]).len().max(1)
        } else {
            value.split('\n').count().max(1)
        };
        vm.set(id, "maxscroll", Value::Number(lines as f64))?;
        // The field's authored height (core_menu centres its DLC label with `_height`).
        // NOT RETAIL YET: retail's autosize height (line height vs field) is not decoded; single-line labels
        // keep the field height here.
        vm.set(id, "_height", Value::Number((bounds[3] - bounds[1]) as f64))?;
        let autosize = vm.get(id, "autoSize").text();
        vm.set(
            id,
            "_width",
            Value::Number(
                if autosize == "left" || autosize == "right" || autosize == "center" {
                    width
                } else {
                    bounds[2] - bounds[0]
                } as f64,
            ),
        )?;
        Ok(())
    }
    /// Queues every handler of `events` whose flags include `event` (record order, as the
    /// retail loop in sub_82E5B158 walks the table).
    fn queue_events(&mut self, object: usize, events: &[ClipAction], event: u32, now: bool) {
        for e in events
            .iter()
            .filter(|e| e.flags & event != 0 && e.actions_offset != 0)
        {
            let p = Pending {
                object,
                offset: e.actions_offset,
                event,
            };
            if now {
                self.immediate.push_back(p);
            } else if event == clip_event::ENTER_FRAME {
                self.pending.push_front(p);
            } else {
                self.pending.push_back(p);
            }
        }
    }
    /// Next action stream to run: immediate handlers first, then the queue. Frame actions
    /// and handlers of removed clips are skipped, except unload, which retail queues during
    /// removal (sub_82E55DA8) and runs on the removed clip.
    pub fn next_action(&mut self) -> Option<Pending> {
        loop {
            let p = self
                .immediate
                .pop_front()
                .or_else(|| self.pending.pop_front())?;
            if p.event == clip_event::UNLOAD || self.instances.contains_key(&p.object) {
                return Some(p);
            }
        }
    }
    /// Removes every clip of this movie (unload handlers queued as for a removal).
    pub fn unload(&mut self, vm: &mut Vm) {
        if self.root != usize::MAX {
            self.remove(vm, self.root);
        }
    }
    fn remove(&mut self, vm: &mut Vm, id: usize) {
        self.attached.remove(&id);
        self.scaled.remove(&id);
        if let Some(instance) = self.instances.remove(&id) {
            // Retail sub_82E55DA8 dispatches unload before tearing down the children.
            if let Some(p) = &instance.placement {
                let events = p.clip_actions.clone();
                self.queue_events(id, &events, clip_event::UNLOAD, false);
            }
            for child in instance.children.values() {
                self.remove(vm, *child);
            }
        }
        // Retired handles can remain referenced by ActionScript, but no longer
        // participate in timeline advancement or rendering.
        let _ = vm.set(id, "_visible", Value::Bool(false));
    }
    pub fn seek(
        &mut self,
        vm: &mut Vm,
        id: usize,
        frame: usize,
        nesting: usize,
    ) -> Result<(), String> {
        let instance = self.instances.get_mut(&id).ok_or("APT absent movie instance")?;
        // skate_vita (performance): the children map is updated in place (upstream cloned it and built a new one
        // every frame); nothing reads the parent's map while its frame is placed (`create` does not).
        let (instance_character, mut children, cursor, settled) =
            (instance.character, std::mem::take(&mut instance.children), instance.cursor.take(), instance.settled.take());
        let characters = Arc::clone(&self.characters);
        let states = Arc::clone(&self.states);
        let character = characters.get(&instance_character).ok_or("APT absent movie character")?;
        if frame >= character.frames.len() {
            return Err(format!(
                "APT frame {frame} outside character {}",
                character.id
            ));
        }
        let frame_count = character.frames.len();
        let actions: Vec<_> = character.frames[frame]
            .controls
            .iter()
            .filter(|c| c.type_name == "do_action" && c.actions_offset != 0)
            .map(|c| c.actions_offset)
            .collect();
        // skate_vita (performance): the next frame of a settled clip with no controls places the same list over the same
        // children: of the placement only the children's name fields are written again (a script may have changed them).
        let unchanged_frame = matches!(&cursor, Some((at, _)) if frame == at + 1) && character.frames[frame].controls.is_empty();
        let list = match cursor {
            Some((at, mut list)) if frame == at + 1 => {
                for control in &character.frames[frame].controls {
                    list.apply(control)?;
                }
                std::borrow::Cow::Owned(list)
            }
            Some((at, list)) if frame == at => std::borrow::Cow::Owned(list),
            _ => display_list(character, &states[&instance_character], frame)?,
        };
        // An import no library resolved stays absent, like retail's null slot: nothing is placed (the central HUD's
        // team logo loader, `Image_Renderer_Component`, without the front end's image_renderer movie).
        // skate_vita (performance): `!characters.contains_key(c) && linkage has c` through the set of such characters
        // (usually empty: no lookup per child).
        let unresolved = self.unresolved_characters();
        let absent = |p: &Placement| !unresolved.is_empty() && unresolved.contains(&p.character);
        let fast = unchanged_frame && settled.is_some_and(|s| Arc::ptr_eq(&s, &unresolved));
        let mut placed = 0;
        for (&depth, placement) in list.depths.iter().filter(|_| fast) {
            if absent(placement) || placement.name.is_empty() {
                continue;
            }
            let child = *children.get(&depth).ok_or("APT settled clip without its child")?;
            vm.set(id, &placement.name, Value::Object(child))?;
        }
        for (&depth, placement) in list.depths.iter().filter(|_| !fast) {
            if absent(placement) {
                continue;
            }
            let previous = children.get(&depth).copied();
            let stale = previous
                .and_then(|old| self.instances.get(&old))
                .and_then(|i| i.placement.as_ref())
                .map(|p| p.name.as_str())
                .filter(|name| !name.is_empty() && *name != placement.name);
            if let Some(name) = stale {
                vm.objects[id].fields.remove(name);
            }
            let child = if let Some(old) = previous.filter(|old| {
                self.instances
                    .get(old)
                    .is_some_and(|i| i.character == placement.character)
            }) {
                old
            } else {
                if let Some(old) = previous {
                    self.remove(vm, old);
                }
                self.create(
                    vm,
                    placement.character,
                    Some(id),
                    nesting + 1,
                    &placement.clip_actions,
                )?
            };
            let old = self.instances[&child].placement.as_ref();
            let unchanged = old == Some(placement);
            if old.is_none_or(|old| old.matrix != placement.matrix) && !self.script_moved.contains(&child) {
                vm.set(child, "_x", Value::Number(placement.matrix[4] as f64))?;
                vm.set(child, "_y", Value::Number(placement.matrix[5] as f64))?;
            }
            if old.is_none_or(|old| old.matrix != placement.matrix) && !self.scaled.contains_key(&child) {
                seed_scale(vm, child, &placement.matrix)?;
            }
            if !placement.name.is_empty() {
                vm.set(id, &placement.name, Value::Object(child))?;
            }
            if !unchanged {
                let slot = &mut self.instances.get_mut(&child).unwrap().placement;
                match slot {
                    Some(old) => old.assign(placement),
                    None => *slot = Some(placement.clone()),
                }
            }
            children.insert(depth, child);
            placed += 1;
        }
        // The previous frame's children this frame does not place: an attached clip stays, the others go. (When the
        // map holds only the depths just placed, there are none.)
        let gone: Vec<(i32, usize)> = if fast || children.len() == placed {
            Vec::new()
        } else {
            children
                .iter()
                .filter(|(depth, child)| !list.depths.get(depth).is_some_and(|p| !absent(p)) && !self.attached.contains(child))
                .map(|(d, c)| (*d, *c))
                .collect()
        };
        for (depth, child) in gone {
            children.remove(&depth);
            if let Some(name) = self.instances.get(&child).and_then(|i| i.placement.as_ref()).map(|p| p.name.as_str()) {
                if !name.is_empty() {
                    vm.objects[id].fields.remove(name);
                }
            }
            self.remove(vm, child);
        }
        let current = self.instances.get_mut(&id).unwrap();
        current.frame = frame;
        current.children = children;
        current.settled = Some(unresolved);
        if frame_count > 1 {
            current.cursor = Some((frame, list.into_owned()));
        }
        vm.set(id, "_currentframe", Value::Number((frame + 1) as f64))?;
        vm.set(id, "_totalframes", Value::Number(frame_count as f64))?;
        for offset in actions {
            self.pending.push_back(Pending {
                object: id,
                offset,
                event: 0,
            });
        }
        Ok(())
    }
    /// Something changed the children of `id`: its next frame is placed in full.
    fn unsettle(&mut self, id: usize) {
        if let Some(instance) = self.instances.get_mut(&id) {
            instance.settled = None;
        }
    }
    /// The characters a placement leaves absent (see `unresolved`), computed when needed.
    fn unresolved_characters(&mut self) -> Arc<std::collections::BTreeSet<i32>> {
        let characters = &self.characters;
        Arc::clone(self.unresolved.get_or_insert_with(|| {
            Arc::new(self.linkage.values().copied().filter(|c| !characters.contains_key(c)).collect())
        }))
    }
    pub fn advance(&mut self, vm: &mut Vm) -> Result<(), String> {
        // Retail sub_82E5B640 ticks every clip (playing or not) and dispatches enterFrame
        // from the second tick on; clips placed during this advance get load instead.
        self.ticks += 1;
        let tick = self.ticks;
        let playing: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, i)| i.playing)
            .map(|(id, i)| (*id, i.character, i.frame))
            .collect();
        let seek = crate::fe::profile::span(crate::fe::profile::Span::Seek);
        for (id, character, frame) in playing {
            if !self.instances.contains_key(&id) {
                continue;
            }
            let count = self.characters[&character].frames.len();
            if count > 1 {
                self.seek(vm, id, (frame + 1) % count, 0)?;
            }
        }
        drop(seek);
        // Each tick pushes its enterFrame to the queue front (sub_82E78240) and the parent
        // ticks before its children (sub_82E7C658), so children's handlers run first; ids
        // grow with creation, so pushing in id order gives the same result.
        // skate_vita (performance): the clips that existed before this advance (`born`), in id order, their handlers
        // queued as `queue_events(.., ENTER_FRAME, false)` does.
        for (&id, instance) in self.instances.iter() {
            if instance.born == tick {
                continue;
            }
            let Some(p) = instance.placement.as_ref().filter(|p| !p.clip_actions.is_empty()) else {
                continue;
            };
            if self.characters[&instance.character].type_name == "sprite" {
                for e in p.clip_actions.iter().filter(|e| e.flags & clip_event::ENTER_FRAME != 0 && e.actions_offset != 0) {
                    self.pending.push_front(Pending { object: id, offset: e.actions_offset, event: clip_event::ENTER_FRAME });
                }
            }
        }
        Ok(())
    }
    /// MovieClip.attachMovie(linkage, name, depth[, init]): a new clip of the exported (or imported) symbol under
    /// `parent`, named, at a script depth, with the init object's fields copied in (standard AS2 semantics; NOT
    /// RETAIL YET: the retail handler is not decoded). `None` when the symbol or the parent is unknown.
    pub fn attach(
        &mut self,
        vm: &mut Vm,
        parent: usize,
        linkage: &str,
        name: &str,
        depth: i32,
        init: Option<usize>,
    ) -> Result<Option<usize>, String> {
        let (Some(&character), true) = (self.linkage.get(linkage), self.instances.contains_key(&parent)) else {
            return Ok(None);
        };
        self.unsettle(parent);
        let key = depth.saturating_add(SCRIPT_DEPTH);
        if let Some(old) = self.instances[&parent].children.get(&key).copied() {
            let old_name = vm.get(old, "_name").text();
            if vm.get(parent, &old_name) == Value::Object(old) {
                vm.objects[parent].fields.remove(old_name.as_str());
            }
            self.remove(vm, old);
        }
        let mut nesting = 1;
        let mut up = vm.get(parent, "_parent");
        while let Value::Object(p) = up {
            nesting += 1;
            if nesting > 32 {
                break;
            }
            up = vm.get(p, "_parent");
        }
        let child = self.create(vm, character, Some(parent), nesting, &[])?;
        vm.set(child, "_name", Value::Text(name.into()))?;
        if !name.is_empty() {
            vm.set(parent, name, Value::Object(child))?;
        }
        if let Some(init) = init {
            let fields: Vec<(crate::fe::apt_vm::Name, Value)> = vm.objects[init].fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for (k, v) in fields {
                vm.set(child, &k, v)?;
            }
        }
        seed_scale(vm, child, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0])?;
        self.instances.get_mut(&child).unwrap().placement = Some(Placement {
            character,
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            color: [255, 255, 255, 255, 0, 0, 0, 0],
            ratio: 0.0,
            name: name.into(),
            clip_depth: -1,
            blend_mode: 0,
            clip_actions: Vec::new(),
        });
        self.instances.get_mut(&parent).unwrap().children.insert(key, child);
        self.attached.insert(child);
        Ok(Some(child))
    }

    pub fn method(
        &mut self,
        vm: &mut Vm,
        id: usize,
        method: &str,
        args: &[Value],
    ) -> Result<bool, String> {
        if !self.instances.contains_key(&id) {
            return Ok(false);
        }
        match method {
            "removeMovieClip" if self.attached.contains(&id) => {
                let name = vm.get(id, "_name").text();
                if let Value::Object(parent) = vm.get(id, "_parent") {
                    if vm.get(parent, &name) == Value::Object(id) {
                        vm.objects[parent].fields.remove(name.as_str());
                    }
                    if let Some(instance) = self.instances.get_mut(&parent) {
                        instance.children.retain(|_, c| *c != id);
                    }
                    self.unsettle(parent);
                }
                self.remove(vm, id);
            }
            // MovieClip.createTextField(name, depth, x, y, width, height): a new, empty text field at a script depth
            // (sk8popup's body). Standard AS2 semantics, NOT RETAIL YET (the retail handler is not decoded): its own
            // text descriptor (font from setTextFormat / setNewTextFormat, else the FontManager fallback at 12 units,
            // left aligned, white), placed at (x, y), width x height bounds.
            "createTextField" => {
                self.unsettle(id);
                let text = |i: usize| args.get(i).map(Value::text).unwrap_or_default();
                let number = |i: usize| args.get(i).map_or(0.0, Value::number) as f32;
                let (name, depth) = (text(0), args.get(1).map_or(0, crate::fe::apt_vm::to_integer) as i32);
                let font = self.text_assets.font_named("").ok_or("createTextField: the set has no fonts")?;
                let character = SCRIPT_TEXT_BASE - self.characters.keys().filter(|k| **k <= SCRIPT_TEXT_BASE).count() as i32;
                self.unresolved = None;
                Arc::make_mut(&mut self.characters).insert(
                    character,
                    Character {
                        id: character,
                        type_name: "text".into(),
                        frames: Vec::new(),
                        text: Some(serde_json::json!({
                            "font_id": font, "font_height": 12.0, "color_argb": "#FFFFFFFF", "alignment": 0,
                            "word_wrap": false, "initial_text": "",
                        })),
                        bounds: Some([0.0, 0.0, number(4), number(5)]),
                    },
                );
                let parent = id;
                let key = depth.saturating_add(SCRIPT_DEPTH);
                if let Some(old) = self.instances[&parent].children.get(&key).copied() {
                    self.remove(vm, old);
                }
                let child = self.create(vm, character, Some(parent), 1, &[])?;
                vm.set(child, "_name", Value::Text(name.clone()))?;
                if !name.is_empty() {
                    vm.set(parent, &name, Value::Object(child))?;
                }
                let matrix = [1.0, 0.0, 0.0, 1.0, number(2), number(3)];
                seed_scale(vm, child, &matrix)?;
                vm.set(child, "_x", Value::Number(f64::from(number(2))))?;
                vm.set(child, "_y", Value::Number(f64::from(number(3))))?;
                self.instances.get_mut(&child).unwrap().placement = Some(Placement {
                    character,
                    matrix,
                    color: [255, 255, 255, 255, 0, 0, 0, 0],
                    ratio: 0.0,
                    name,
                    clip_depth: -1,
                    blend_mode: 0,
                    clip_actions: Vec::new(),
                });
                self.instances.get_mut(&parent).unwrap().children.insert(key, child);
                self.attached.insert(child);
                self.script_moved.insert(child);
                return Ok(true);
            }
            "stop" => self.instances.get_mut(&id).unwrap().playing = false,
            "play" => self.instances.get_mut(&id).unwrap().playing = true,
            "gotoAndPlay" | "gotoAndStop" => {
                let c = &self.characters[&self.instances[&id].character];
                let frame = match args.first().ok_or("APT goto requires frame")? {
                    // An unknown label leaves the clip where it is (Flash behaviour; retail not traced), so a
                    // menu item with an icon name the movie lacks does not stop the whole movie.
                    Value::Text(label) => match c.frames.iter().position(|f| {
                        f.controls.iter().any(|control| {
                            control.type_name == "frame_label" && control.label.as_deref() == Some(label)
                        })
                    }) {
                        Some(frame) => frame,
                        None => return Ok(true),
                    },
                    value => {
                        let frame = value.number();
                        if !frame.is_finite() || frame < 0.0 {
                            return Err(format!(
                                "Invalid APT frame {frame} in {} on character {}",
                                method, c.id
                            ));
                        }
                        // The shipped line timer deliberately requests zero at
                        // full capacity. Frame zero addresses the first frame.
                        (frame as usize).saturating_sub(1)
                    }
                };
                self.seek(vm, id, frame, 0)?;
                self.instances.get_mut(&id).unwrap().playing = method == "gotoAndPlay";
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// Cross-movie imports (doc 31, Milestone 3b): an imported character id gets a copy of the
/// defining library movie's character, together with every character it places, its action
/// streams and its fonts, under ids that are free in this movie. Copies are made in binding
/// order with a deterministic id / action-offset allocator, so the same movies always give the
/// same ids.
/// NOT RETAIL (layout only): retail sub_82E76070 stores a pointer to the source movie's
/// character in this movie's character table and keeps a reference to the source movie
/// (for its constants); copying keeps our single-movie player unchanged and behaves the same,
/// because character data is immutable and our action streams are already decoded (no
/// constant pool lookups). Unresolved imports stay absent, like retail's null slot.
impl Movie {
    pub fn link_imports(
        &mut self,
        key: &str,
        resolution: &crate::fe::apt_imports::Resolution,
        libraries: &BTreeMap<String, Movie>,
    ) -> Result<usize, String> {
        let key = crate::fe::apt_imports::library_key(key);
        let mut memo = BTreeMap::new();
        let mut linked = 0;
        for b in resolution.bindings.iter().filter(|b| b.movie == key) {
            if self.characters.contains_key(&b.character_id) {
                return Err(format!(
                    "APT import {} collides with a local character",
                    b.character_id
                ));
            }
            let library = libraries
                .get(&b.source)
                .ok_or_else(|| format!("APT library {} is not loaded", b.source))?;
            let mut link = Link {
                resolution,
                libraries,
                memo: &mut memo,
            };
            self.copy_character(
                &mut link,
                &b.source,
                library,
                b.source_character,
                Some(b.character_id),
                0,
            )?;
            linked += 1;
        }
        // Every library a copy came from keeps its exported symbols reachable by linkage name, placed or not:
        // gesture_item's SetGestureType attaches "g_" + the trick's gesture (g_ollie_kickflip...), symbols its own
        // timeline never places. The movie's own names and earlier copies win. NOT RETAIL YET: retail's
        // attachMovie lookup is not decoded; this gives the Flash rule (a clip attaches from its defining movie).
        let touched: std::collections::BTreeSet<String> = memo.keys().map(|(lib, _)| lib.clone()).collect();
        for lib_key in touched {
            let Some(library) = libraries.get(&lib_key) else {
                continue;
            };
            let missing: Vec<i32> =
                library.linkage.iter().filter(|(name, _)| !self.linkage.contains_key(*name)).map(|(_, c)| *c).collect();
            let mut link = Link {
                resolution,
                libraries,
                memo: &mut memo,
            };
            for character in missing {
                self.copy_character(&mut link, &lib_key, library, character, None, 0)?;
            }
        }
        Ok(linked)
    }
    fn fresh_character(&self) -> i32 {
        self.characters.keys().next_back().map_or(1, |k| k + 1)
    }
    fn fresh_action(&self) -> u32 {
        self.actions
            .keys()
            .filter_map(|k| k.parse::<u32>().ok())
            .max()
            .map_or(1, |k| k + 1)
    }
    fn copy_character(
        &mut self,
        link: &mut Link,
        lib_key: &str,
        lib: &Movie,
        source: i32,
        as_id: Option<i32>,
        depth: usize,
    ) -> Result<i32, String> {
        if depth > 64 || self.characters.len() > 65_536 {
            return Err("APT import copy limit".into());
        }
        if as_id.is_none() {
            if let Some(id) = link.memo.get(&(lib_key.to_string(), source)) {
                return Ok(*id);
            }
        }
        let Some(original) = lib.characters.get(&source) else {
            // Imported again inside the library: follow its own binding.
            let b = link
                .resolution
                .binding(lib_key, source)
                .ok_or_else(|| format!("APT {lib_key} character {source} is unresolved"))?
                .clone();
            let next = link
                .libraries
                .get(&b.source)
                .ok_or_else(|| format!("APT library {} is not loaded", b.source))?;
            let id =
                self.copy_character(link, &b.source, next, b.source_character, as_id, depth + 1)?;
            link.memo.insert((lib_key.to_string(), source), id);
            return Ok(id);
        };
        let id = as_id.unwrap_or_else(|| self.fresh_character());
        link.memo.entry((lib_key.to_string(), source)).or_insert(id);
        // The library's linkage names travel with the copy, so a component's nested clips (highlight_option's
        // HighlightText) get the classes registered for them; the movie's own names win.
        for (name, character) in &lib.linkage {
            if *character == source {
                self.unresolved = None;
                self.linkage.entry(name.clone()).or_insert(id);
                self.class_links.insert(id, name.clone());
            }
        }
        let mut c = original.clone();
        c.id = id;
        if c.type_name == "shape" {
            let origin = lib.shape_origins.get(&source).cloned();
            self.shape_origins
                .insert(id, origin.unwrap_or_else(|| (lib_key.to_string(), source)));
        }
        // Reserve the id before copying children so they allocate past it.
        self.unresolved = None;
        Arc::make_mut(&mut self.characters).insert(id, c.clone());
        for frame in &mut c.frames {
            for control in &mut frame.controls {
                let placed = matches!(
                    control.type_name.as_str(),
                    "place_object2" | "place_object3"
                ) && control.flags & 2 != 0;
                if let Some(child) = control.character_id.filter(|_| placed) {
                    control.character_id =
                        Some(self.copy_character(link, lib_key, lib, child, None, depth + 1)?);
                }
                for event in &mut control.clip_actions {
                    let code = lib
                        .actions
                        .get(&event.actions_offset.to_string())
                        .ok_or("APT library clip action stream missing")?
                        .clone();
                    let offset = self.fresh_action();
                    Arc::make_mut(&mut self.actions).insert(offset.to_string(), code);
                    event.actions_offset = offset;
                }
                if control.actions_offset != 0
                    && matches!(control.type_name.as_str(), "do_action" | "do_init_action")
                {
                    let code = lib
                        .actions
                        .get(&control.actions_offset.to_string())
                        .ok_or("APT library action stream missing")?
                        .clone();
                    let offset = self.fresh_action();
                    Arc::make_mut(&mut self.actions).insert(offset.to_string(), code);
                    control.actions_offset = offset;
                }
            }
        }
        if let Some(font) = c.text.as_ref().and_then(|t| t["font_id"].as_i64()) {
            let new = self.copy_character(link, lib_key, lib, font as i32, None, depth + 1)?;
            if let Some(text) = &mut c.text {
                text["font_id"] = serde_json::json!(new);
            }
        }
        if let Some(font) = lib.text_assets.fonts.get(&source) {
            // The secondary native font travels with the font (it is not an APT character).
            self.text_assets.fonts.insert(id, font.clone());
        }
        Arc::make_mut(&mut self.states).insert(id, display_states(&c)?);
        self.unresolved = None;
        Arc::make_mut(&mut self.characters).insert(id, c);
        Ok(id)
    }
}
struct Link<'a> {
    resolution: &'a crate::fe::apt_imports::Resolution,
    libraries: &'a BTreeMap<String, Movie>,
    memo: &'a mut BTreeMap<(String, i32), i32>,
}

#[cfg(test)]
mod import_tests {
    use super::*;
    use crate::fe::apt_imports::{resolve, Import, MovieDecl};

    fn place(depth: i32, character: i32) -> serde_json::Value {
        serde_json::json!({"type_name": "place_object2", "flags": 2, "depth": depth, "character_id": character})
    }

    #[test]
    fn apt_imports_link_copies_library_subtree() {
        // Library: export "Button" = sprite 3 placing shape 1 and running action 100.
        let lib_json = serde_json::json!({
            "characters": [
                {"id": 0, "type_name": "animation", "frames": [{"controls": []}]},
                {"id": 1, "type_name": "shape"},
                {"id": 3, "type_name": "sprite", "frames": [{"controls": [
                    place(1, 1),
                    {"type_name": "do_action", "actions_offset": 100}
                ]}]}
            ],
            "actions": {"100": []},
            "language": {},
            "exports": [{"name": "Button", "character_id": 3}]
        });
        let lib = Movie::load(&lib_json).unwrap();
        let mut root = Movie::load(&serde_json::json!({
            "characters": [{"id": 0, "type_name": "animation", "frames": [{"controls": [place(1, 9)]}]}],
            "actions": {"7": []},
            "language": {}
        }))
        .unwrap();
        let root_decl = MovieDecl {
            imports: vec![Import {
                file: "source/controls/lib".into(),
                name: "Button".into(),
                character_id: 9,
            }],
            exports: vec![],
            characters: [0].into(),
        };
        let mut decls = BTreeMap::new();
        decls.insert(
            "source/controls/lib".to_string(),
            MovieDecl::from_json(&lib_json).unwrap(),
        );
        let r = resolve("root", root_decl, &mut decls);
        let mut libs = BTreeMap::new();
        libs.insert("source/controls/lib".to_string(), lib);
        assert_eq!(root.link_imports("root", &r, &libs).unwrap(), 1);
        let imported = &root.characters[&9];
        assert_eq!(imported.type_name, "sprite");
        let child = imported.frames[0].controls[0].character_id.unwrap();
        assert_eq!(root.characters[&child].type_name, "shape");
        let offset = imported.frames[0].controls[1].actions_offset;
        assert!(offset != 100 && root.actions.contains_key(&offset.to_string()));
        // The imported clip now instantiates like a local one.
        let mut vm = Vm::new();
        root.initialize(&mut vm).unwrap();
        assert!(root.instances.values().any(|i| i.character == child));
        // Linking twice or without the library is an error, not a panic.
        assert!(root.link_imports("root", &r, &BTreeMap::new()).is_err());
    }
}

#[cfg(test)]
mod clip_action_tests {
    use super::*;
    use crate::fe::apt_display::clip_event as ev;
    use crate::fe::apt_vm::Host;

    struct Stub;
    impl Host for Stub {
        fn call(&mut self, _: &mut Vm, _: usize, _: &str, _: Vec<Value>) -> Result<Value, String> {
            Ok(Value::Undefined)
        }
    }

    fn events() -> serde_json::Value {
        // One handler per event kind, in retail record order; offsets name the event.
        serde_json::json!([
            {"flags": ev::LOAD, "key_code": 0, "actions_offset": 11},
            {"flags": ev::ENTER_FRAME, "key_code": 0, "actions_offset": 12},
            {"flags": ev::UNLOAD, "key_code": 0, "actions_offset": 14},
            {"flags": ev::CONSTRUCT, "key_code": 0, "actions_offset": 40},
            {"flags": ev::INITIALIZE, "key_code": 0, "actions_offset": 20},
            {"flags": ev::PRESS, "key_code": 0, "actions_offset": 30}
        ])
    }

    /// Root: frame 0 places sprite 2 at depth 1 with clip actions (named "clip"), frame 1
    /// removes it. Sprite 2: one frame with frame action 50.
    fn movie() -> Movie {
        let set_type = serde_json::json!([
            {"offset": 0, "opcode": 0xa1, "operand": "_type"},
            {"offset": 1, "opcode": 0xa6, "operand": "Button"}
        ]);
        Movie::load(&serde_json::json!({
            "characters": [
                {"id": 0, "type_name": "movie", "frames": [
                    {"controls": [{"type_name": "place_object2", "flags": 0x80 | 0x20 | 2,
                        "depth": 1, "character_id": 2, "name": "clip", "actions_offset": 999,
                        "clip_actions": events()}]},
                    {"controls": [{"type_name": "remove_object2", "depth": 1}]}
                ]},
                {"id": 2, "type_name": "sprite", "frames": [
                    {"controls": [{"type_name": "do_action", "actions_offset": 50}]}
                ]}
            ],
            "actions": {"11": [], "12": [], "14": [], "20": [], "30": [], "50": [], "40": set_type},
            "language": {}
        }))
        .unwrap()
    }

    fn drain(m: &mut Movie) -> Vec<(u32, bool)> {
        let mut out = Vec::new();
        while let Some(p) = m.next_action() {
            out.push((p.offset, m.instances.contains_key(&p.object)));
        }
        out
    }

    #[test]
    fn apt_clip_actions_dispatch_in_retail_order() {
        let mut m = movie();
        let mut vm = Vm::new();
        m.initialize(&mut vm).unwrap();
        // Placement: initialize then construct right away, then the clip's frame action,
        // then load. Press waits for input; nothing else fires.
        assert_eq!(
            drain(&mut m),
            [(20, true), (40, true), (50, true), (11, true)]
        );
        // Next tick: enterFrame at the queue front, before the frame actions queued by the
        // same tick (the root is stopped on frame 0 here, so only the handler runs).
        m.instances.get_mut(&m.root).unwrap().playing = false;
        m.advance(&mut vm).unwrap();
        assert_eq!(drain(&mut m), [(12, true)]);
        // Removal queues unload, which still runs although the clip is gone.
        let root = m.root;
        m.seek(&mut vm, root, 1, 0).unwrap();
        assert_eq!(drain(&mut m), [(14, false)]);
        // Removed clips get no more handlers.
        m.advance(&mut vm).unwrap();
        assert!(drain(&mut m).iter().all(|(o, _)| *o != 12));
    }

    #[test]
    fn apt_clip_action_runs_on_its_clip() {
        let mut m = movie();
        let mut vm = Vm::new();
        m.initialize(&mut vm).unwrap();
        let Value::Object(clip) = vm.get(m.root, "clip") else {
            panic!("named clip missing")
        };
        while let Some(p) = m.next_action() {
            let code = m.actions[&p.offset.to_string()].clone();
            vm.begin_update();
            vm.run_on(p.object, &code, &mut Stub).unwrap();
        }
        // Construct (0xa6 = push string + setVariable) set the member on the clip itself.
        assert_eq!(vm.get(clip, "_type").text(), "Button");
        assert!(matches!(vm.get(m.root, "_type"), Value::Undefined));
    }

    #[test]
    fn apt_clip_actions_unsafe_tables_are_errors() {
        let place = |extra: serde_json::Value| {
            let mut c = serde_json::json!({"type_name": "place_object2", "flags": 0x82,
                "depth": 1, "character_id": 2, "actions_offset": 64});
            c.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<Control>(c).unwrap()
        };
        let mut list = DisplayList::default();
        // Exported before 3d (pointer but no decoded table): refused, not ignored.
        assert!(list.apply(&place(serde_json::json!({}))).is_err());
        let many: Vec<_> = (0..65)
            .map(|i| serde_json::json!({"flags": 1, "actions_offset": i + 1}))
            .collect();
        assert!(list
            .apply(&place(serde_json::json!({"clip_actions": many})))
            .is_err());
        let one = serde_json::json!({"clip_actions": [{"flags": 1, "actions_offset": 7}]});
        list.apply(&place(one)).unwrap();
        assert_eq!(list.depths[&1].clip_actions[0].actions_offset, 7);
        // A move without flag 0x80 keeps the table (retail stores it only when present).
        let mv = serde_json::json!({"type_name": "place_object2", "flags": 1, "depth": 1});
        list.apply(&serde_json::from_value(mv).unwrap()).unwrap();
        assert_eq!(list.depths[&1].clip_actions.len(), 1);
    }

    /// Doc 31, Milestone 3f: retail sub_82E7B340 / sub_82E7A950 clip-depth storage.
    #[test]
    fn apt_clip_depth_follows_retail_placement() {
        let apply = |list: &mut DisplayList, c: serde_json::Value| {
            list.apply(&serde_json::from_value::<Control>(c).unwrap())
        };
        let mut list = DisplayList::default();
        // New object: the record's value is stored even without flag 0x40.
        apply(&mut list, serde_json::json!({"type_name": "place_object2", "flags": 2,
            "depth": 1, "character_id": 2, "clip_depth": 5})).unwrap();
        assert_eq!(list.depths[&1].clip_depth, 5);
        // Move of an existing object: retail passes -1 and keeps the stored value.
        apply(&mut list, serde_json::json!({"type_name": "place_object2", "flags": 0x41,
            "depth": 1, "clip_depth": 9})).unwrap();
        assert_eq!(list.depths[&1].clip_depth, 5);
        apply(&mut list, serde_json::json!({"type_name": "place_object2", "flags": 0x41,
            "depth": 1})).unwrap();
        assert_eq!(list.depths[&1].clip_depth, 5);
        // Replacing the character creates the object again with the record's value.
        apply(&mut list, serde_json::json!({"type_name": "place_object2", "flags": 3,
            "depth": 1, "character_id": 3, "clip_depth": -1})).unwrap();
        assert_eq!(list.depths[&1].clip_depth, -1);
        // Stored as a 16-bit halfword (sth props+22); a missing value (old export) is none.
        apply(&mut list, serde_json::json!({"type_name": "place_object2", "flags": 2,
            "depth": 2, "character_id": 2, "clip_depth": 0x1_0007})).unwrap();
        assert_eq!(list.depths[&2].clip_depth, 7);
        apply(&mut list, serde_json::json!({"type_name": "place_object2", "flags": 2,
            "depth": 3, "character_id": 2})).unwrap();
        assert_eq!(list.depths[&3].clip_depth, -1);
        // Removal drops it with the object.
        apply(&mut list, serde_json::json!({"type_name": "remove_object2", "depth": 2})).unwrap();
        assert!(!list.depths.contains_key(&2));
    }
}
